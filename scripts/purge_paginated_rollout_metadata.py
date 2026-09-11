#!/usr/bin/env python3
"""Find and remove non-ordinal-zero SessionMeta records from paginated rollouts."""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
from typing import Iterator


def rollout_files(codex_home: Path) -> Iterator[Path]:
    seen_plain: set[Path] = set()
    candidates = []
    for directory_name in ("sessions", "archived_sessions"):
        directory = codex_home / directory_name
        if not directory.exists():
            continue
        candidates.extend(
            path
            for path in directory.rglob("rollout-*")
            if path.is_file() and path.name.endswith((".jsonl", ".jsonl.zst"))
        )
    for path in sorted(candidates):
        if path.suffix == ".zst" and path.with_suffix("") in seen_plain:
            continue
        if path.suffix == ".jsonl":
            seen_plain.add(path)
        yield path


def read_rollout(path: Path) -> bytes:
    if path.suffix != ".zst":
        return path.read_bytes()
    result = subprocess.run(
        ["zstd", "--quiet", "--decompress", "--stdout", str(path)],
        check=False,
        capture_output=True,
    )
    if result.returncode:
        raise RuntimeError(
            f"zstd could not decompress {path}: {result.stderr.decode(errors='replace')}"
        )
    return result.stdout


def write_rollout(path: Path, contents: bytes, mode: int) -> None:
    temporary = tempfile.NamedTemporaryFile(
        dir=path.parent, prefix=f".{path.name}.", delete=False
    )
    temporary_path = Path(temporary.name)
    try:
        if path.suffix == ".zst":
            result = subprocess.run(
                ["zstd", "--quiet", "--stdout"],
                input=contents,
                stdout=temporary,
                check=False,
                stderr=subprocess.PIPE,
            )
            if result.returncode:
                raise RuntimeError(
                    f"zstd could not compress {path}: {result.stderr.decode(errors='replace')}"
                )
        else:
            temporary.write(contents)
        temporary.flush()
        os.fsync(temporary.fileno())
        temporary.close()
        os.chmod(temporary_path, mode)
        os.replace(temporary_path, path)
    except BaseException:
        temporary.close()
        temporary_path.unlink(missing_ok=True)
        raise


def records(contents: bytes) -> Iterator[tuple[bytes, dict | None]]:
    for line in contents.splitlines(keepends=True):
        try:
            yield line, json.loads(line)
        except (UnicodeDecodeError, json.JSONDecodeError):
            yield line, None


def session_meta(record: dict | None) -> tuple[int | None, dict] | None:
    if not record or record.get("type") != "session_meta":
        return None
    payload = record.get("payload")
    if not isinstance(payload, dict):
        return None
    ordinal = record.get("ordinal")
    return ordinal if isinstance(ordinal, int) else None, payload


def inspect(path: Path) -> tuple[bool, str | None, int]:
    canonical_id = None
    paginated = False
    offending = 0
    for _, record in records(read_rollout(path)):
        metadata = session_meta(record)
        if metadata is None:
            continue
        ordinal, payload = metadata
        if canonical_id is None:
            canonical_id = payload.get("id") or payload.get("session_id")
            paginated = payload.get("history_mode") == "paginated"
        if paginated and ordinal != 0:
            offending += 1
    return paginated and offending > 0, canonical_id, offending


def purge_contents(contents: bytes) -> tuple[bytes, int]:
    retained = bytearray()
    removed = 0
    for line, record in records(contents):
        metadata = session_meta(record)
        if metadata is not None and metadata[0] != 0:
            removed += 1
            continue
        if removed and record is not None:
            ordinal = record.get("ordinal")
            if isinstance(ordinal, int) and ordinal >= removed:
                line = re.sub(
                    rb'("ordinal"\s*:\s*)\d+',
                    rb'\g<1>' + str(ordinal - removed).encode(),
                    line,
                    count=1,
                )
        retained.extend(line)
    return bytes(retained), removed


def target_path(target: str, affected: list[tuple[Path, str | None, int]]) -> Path:
    requested_path = Path(target).expanduser()
    resolved_requested_path = requested_path.resolve() if requested_path.exists() else None
    by_path = [
        path for path, _, _ in affected if resolved_requested_path == path.resolve()
    ]
    if by_path:
        matches = by_path
    else:
        by_filename = [
            path
            for path, _, _ in affected
            if path.name == target or path.name.removesuffix(".zst") == target
        ]
        matches = by_filename
    if not matches:
        matches = [path for path, thread_id, _ in affected if thread_id == target]
    if not matches:
        raise ValueError(f"no affected paginated rollout matched {target}")
    if len(matches) > 1:
        names = ", ".join(str(path) for path in matches)
        raise ValueError(f"multiple affected rollouts matched {target}; use a filename: {names}")
    return matches[0]


@contextlib.contextmanager
def maintenance_lock(codex_home: Path):
    lock_directory = codex_home / ".tmp"
    lock_directory.mkdir(parents=True, exist_ok=True)
    lock_path = lock_directory / "rollout-maintenance.lock"
    with lock_path.open("a+") as lock_file:
        fcntl.flock(lock_file, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(lock_file, fcntl.LOCK_UN)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")),
        help="Codex home containing sessions/ and archived_sessions/",
    )
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--find", action="store_true", help="list affected rollout files")
    modes.add_argument("--purge", metavar="SESSION_ID_OR_FILENAME", help="purge one rollout")
    args = parser.parse_args()

    try:
        with maintenance_lock(args.codex_home):
            affected = []
            for path in rollout_files(args.codex_home):
                applicable, thread_id, count = inspect(path)
                if applicable:
                    affected.append((path, thread_id, count))
            if args.find:
                for path, _, count in affected:
                    print(f"{path}\t{count}")
                return 0

            path = target_path(args.purge, affected)
            original = read_rollout(path)
            repaired, removed = purge_contents(original)
            if removed == 0:
                raise ValueError(f"no removable records found in {path}")
            write_rollout(path, repaired, path.stat().st_mode & 0o777)
            print(f"removed {removed} non-ordinal-zero SessionMeta record(s) from {path}")
            return 0
    except (OSError, RuntimeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
