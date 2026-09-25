#!/usr/bin/env python3
"""Find and remove raw_response_completed events from Codex rollout files."""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from typing import Iterator


def rollout_files(codex_home: Path) -> Iterator[Path]:
    seen_plain: set[Path] = set()
    candidates = []
    for directory_name in ("sessions", "archived_sessions"):
        directory = codex_home / directory_name
        if directory.exists():
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
            record = json.loads(line)
        except (UnicodeDecodeError, json.JSONDecodeError):
            yield line, None
        else:
            yield line, record if isinstance(record, dict) else None


def is_raw_response_completed(record: dict | None) -> bool:
    return bool(
        record
        and record.get("type") == "event_msg"
        and isinstance(record.get("payload"), dict)
        and record["payload"].get("type") == "raw_response_completed"
    )


def inspect(path: Path) -> tuple[str | None, int, bool]:
    session_id = None
    count = 0
    malformed = False
    for _, record in records(read_rollout(path)):
        if record is None:
            malformed = True
            continue
        if record.get("type") == "session_meta" and isinstance(
            record.get("payload"), dict
        ):
            payload = record["payload"]
            session_id = session_id or payload.get("id") or payload.get("session_id")
        count += is_raw_response_completed(record)
    return session_id, count, malformed


def purge_contents(contents: bytes) -> tuple[bytes, int]:
    parsed = list(records(contents))
    if any(record is None for _, record in parsed):
        raise ValueError("rollout contains malformed JSON; refusing to rewrite it")

    retained = bytearray()
    removed = 0
    for line, record in parsed:
        if is_raw_response_completed(record):
            removed += 1
            continue
        if removed:
            ordinal = record.get("ordinal")
            if isinstance(ordinal, int):
                line = re.sub(
                    rb'("ordinal"\s*:\s*)\d+',
                    rb"\g<1>" + str(ordinal - removed).encode(),
                    line,
                    count=1,
                )
        retained.extend(line)
    return bytes(retained), removed


def target_path(target: str, affected: list[tuple[Path, str | None, int]]) -> Path:
    requested_path = Path(target).expanduser()
    resolved_requested_path = (
        requested_path.resolve() if requested_path.exists() else None
    )
    matches = [
        path for path, _, _ in affected if resolved_requested_path == path.resolve()
    ]
    if not matches:
        matches = [
            path
            for path, _, _ in affected
            if path.name == target or path.name.removesuffix(".zst") == target
        ]
    if not matches:
        matches = [path for path, session_id, _ in affected if session_id == target]
    if not matches:
        raise ValueError(f"no affected rollout matched {target}")
    if len(matches) > 1:
        names = ", ".join(str(path) for path in matches)
        raise ValueError(
            f"multiple affected rollouts matched {target}; use a filename: {names}"
        )
    return matches[0]


@contextlib.contextmanager
def maintenance_lock(codex_home: Path):
    lock_directory = codex_home / ".tmp"
    lock_directory.mkdir(parents=True, exist_ok=True)
    with (lock_directory / "rollout-maintenance.lock").open("a+") as lock_file:
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
    modes.add_argument(
        "--find", action="store_true", help="list affected rollout files"
    )
    modes.add_argument("--purge", metavar="SESSION_ID_OR_FILENAME_OR_PATH")
    parser.add_argument(
        "--apply",
        action="store_true",
        help="rewrite the selected rollout; without this flag, --purge only previews",
    )
    args = parser.parse_args()
    if args.apply and not args.purge:
        parser.error("--apply requires --purge")

    try:
        with maintenance_lock(args.codex_home):
            affected = []
            for path in rollout_files(args.codex_home):
                session_id, count, malformed = inspect(path)
                if count:
                    affected.append((path, session_id, count))
                    if malformed:
                        print(
                            f"warning: malformed JSON in {path}; apply will refuse this file",
                            file=sys.stderr,
                        )
            if args.find:
                for path, _, count in affected:
                    print(f"{path}\t{count}")
                return 0

            path = target_path(args.purge, affected)
            count = next(count for candidate, _, count in affected if candidate == path)
            if not args.apply:
                print(
                    f"would remove {count} raw_response_completed event(s) from {path}; "
                    "pass --apply to rewrite"
                )
                return 0

            repaired, removed = purge_contents(read_rollout(path))
            if removed == 0:
                raise ValueError(f"no removable records found in {path}")
            write_rollout(path, repaired, path.stat().st_mode & 0o777)
            print(f"removed {removed} raw_response_completed event(s) from {path}")
            return 0
    except (OSError, RuntimeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
