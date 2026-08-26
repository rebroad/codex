#!/usr/bin/env python3
"""Print the resolved Rusty V8 crate version from a Cargo lockfile."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path


def read_v8_version(lockfile_path: Path) -> str:
    with lockfile_path.open("rb") as lockfile:
        versions = sorted(
            {
                package["version"]
                for package in tomllib.load(lockfile)["package"]
                if package["name"] == "v8"
            }
        )

    if len(versions) != 1:
        raise ValueError(f"expected exactly one resolved v8 version, found: {versions}")
    return versions[0]


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {Path(sys.argv[0]).name} CARGO_LOCK", file=sys.stderr)
        return 2

    try:
        print(read_v8_version(Path(sys.argv[1])))
    except (OSError, KeyError, tomllib.TOMLDecodeError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
