#!/usr/bin/env python3
"""Update the model Codex restores for existing threads.

Preview changes by default. Pass --apply to update state_5.sqlite. This changes
only the indexed thread model used by resume; rollout history is left intact.
Use --map OLD=NEW to extend or override the default mappings after a model
release. Progress is written to stderr.
"""

from __future__ import annotations

import argparse
import os
import sqlite3
import sys
from collections import Counter
from pathlib import Path


DEFAULT_MAPPINGS = {
    "gpt-5.2": "gpt-6-luna",
    "gpt-5.3": "gpt-6-luna",
    "gpt-5.4": "gpt-6-luna",
    "gpt-5.5": "gpt-6-luna",
    "gpt-5.6": "gpt-6-luna",
    "gpt-5.6-sol": "gpt-6.1-sol",
    "gpt-6-sol": "gpt-6.1-sol",
}
NON_MODEL_IDENTIFIERS = {"codex-auto-review", "chatgpt-window"}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")),
        help="Codex home containing state_5.sqlite",
    )
    parser.add_argument(
        "--map",
        dest="mappings",
        action="append",
        metavar="OLD=NEW",
        help="add or override a model mapping; may be repeated",
    )
    parser.add_argument(
        "--apply", action="store_true", help="update indexed thread models"
    )
    return parser.parse_args()


def make_mappings(overrides: list[str] | None) -> dict[str, str]:
    mappings = dict(DEFAULT_MAPPINGS)
    for item in overrides or []:
        old, separator, new = item.partition("=")
        if not separator or not old or not new:
            raise ValueError(f"invalid mapping {item!r}; expected OLD=NEW")
        mappings[old] = new
    return mappings


def replacement_for(model: str, mappings: dict[str, str]) -> str | None:
    for old in sorted(mappings, key=len, reverse=True):
        if model == old or model.startswith(old + "-"):
            return mappings[old]
    return None


class Progress:
    def __init__(self, total: int) -> None:
        self.total = total
        self.terminal = sys.stderr.isatty()
        self.last_percent = -1

    def update(self, completed: int) -> None:
        if not self.total:
            return
        percent = 100 * completed / self.total
        if not self.terminal and int(percent) == self.last_percent:
            return
        self.last_percent = int(percent)
        label = f"Progress: {completed}/{self.total} ({percent:.1f}%)"
        if self.terminal:
            print(f"\r{label}", end="", file=sys.stderr, flush=True)
        else:
            print(label, file=sys.stderr, flush=True)

    def finish(self) -> None:
        if self.total and self.terminal:
            print(file=sys.stderr, flush=True)


def main() -> int:
    args = parse_args()
    try:
        mappings = make_mappings(args.mappings)
    except ValueError as error:
        print(error, file=sys.stderr)
        return 2

    database = args.codex_home / "state_5.sqlite"
    if not database.is_file():
        print(f"Codex state database does not exist: {database}", file=sys.stderr)
        return 2

    try:
        if args.apply:
            connection = sqlite3.connect(database, timeout=30)
        else:
            connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
        connection.row_factory = sqlite3.Row
        connection.execute("PRAGMA busy_timeout = 30000")
        if args.apply:
            connection.execute("BEGIN IMMEDIATE")
        rows = connection.execute("SELECT id, model FROM threads ORDER BY id").fetchall()
        progress = Progress(len(rows))
        progress.update(0)
        totals: Counter[str] = Counter()
        errors = 0

        for index, row in enumerate(rows, start=1):
            model = row["model"]
            try:
                if not model:
                    totals["no_model"] += 1
                elif model in NON_MODEL_IDENTIFIERS:
                    totals[f"non_model_identifier: {model}"] += 1
                elif model in mappings.values():
                    totals[f"already_at_target: {model}"] += 1
                else:
                    replacement = replacement_for(model, mappings)
                    if replacement is None:
                        totals[f"no_mapping_for: {model}"] += 1
                    elif args.apply:
                        result = connection.execute(
                            "UPDATE threads SET model = ? WHERE id = ? AND model = ?",
                            (replacement, row["id"], model),
                        )
                        if result.rowcount != 1:
                            raise RuntimeError("thread model changed during update")
                        totals[f"{model} -> {replacement}"] += 1
                    else:
                        totals[f"{model} -> {replacement}"] += 1
            except (sqlite3.Error, RuntimeError) as error:
                errors += 1
                print(f"error: thread {row['id']}: {error}", file=sys.stderr)
            finally:
                progress.update(index)

        progress.finish()
        if args.apply and errors:
            connection.rollback()
        elif args.apply:
            connection.commit()
        connection.close()

        for label, count in sorted(totals.items()):
            print(f"{label}: {count}")
        print(f"Threads in state database: {len(rows)}")
        action = "Updated" if args.apply else "Would update"
        print(
            f"{action}: {sum(count for label, count in totals.items() if ' -> ' in label)} "
            f"of {len(rows)} indexed threads"
        )
        if not args.apply:
            print("Dry run only; pass --apply to write changes.")
        return 1 if errors else 0
    except (OSError, sqlite3.Error, RuntimeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
