#!/usr/bin/env python3
"""Synchronize a Git worktree into a build tree without deleting ignored output."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def git(root: str, *args: str, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(
        ["git", "-C", root, *args], capture_output=True, check=False
    )
    if check and result.returncode != 0:
        message = result.stderr.decode(errors="replace").strip()
        raise RuntimeError(message or f"git {' '.join(args)} failed ({result.returncode})")
    return result


def git_path(root: str, name: str) -> str:
    value = git(root, "rev-parse", "--path-format=absolute", "--git-path", name)
    return os.fsdecode(value.stdout.rstrip(b"\n"))


def same_common_git_dir(source: str, target: str) -> bool:
    source_dir = os.fsdecode(git(source, "rev-parse", "--path-format=absolute", "--git-common-dir").stdout.rstrip(b"\n"))
    target_dir = os.fsdecode(git(target, "rev-parse", "--path-format=absolute", "--git-common-dir").stdout.rstrip(b"\n"))
    return os.path.realpath(source_dir) == os.path.realpath(target_dir)


def mark_skip_worktree(target: str, protected_paths: list[bytes]) -> None:
    pathspecs = sorted({os.fsdecode(path.rstrip(b"/")) for path in protected_paths if path})
    if not pathspecs:
        return
    tracked = git(target, "ls-files", "-z", "--", *pathspecs, check=False)
    if tracked.returncode != 0:
        message = tracked.stderr.decode(errors="replace").strip()
        raise RuntimeError(message or "unable to list tracked symlink-target paths")
    tracked_paths = [path for path in tracked.stdout.split(b"\0") if path]
    if tracked_paths:
        git(target, "update-index", "--skip-worktree", "--", *(os.fsdecode(path) for path in tracked_paths))


def ensure_git_link(source: str, target: str) -> bool:
    """Make target a linked worktree when it has no usable Git metadata."""
    target_dot_git = os.path.join(target, ".git")
    inside = git(target, "rev-parse", "--is-inside-work-tree", check=False)
    if inside.returncode == 0 and inside.stdout.strip() == b"true":
        top = git(target, "rev-parse", "--show-toplevel")
        if os.path.realpath(os.fsdecode(top.stdout.rstrip(b"\n"))) == os.path.realpath(target):
            if same_common_git_dir(source, target):
                return True
            raise RuntimeError(f"refusing to sync into an independent Git worktree: {target}")

    if os.path.isdir(target_dot_git):
        raise RuntimeError(f"refusing to replace target Git metadata directory: {target_dot_git}")
    if os.path.lexists(target_dot_git):
        os.unlink(target_dot_git)

    parent = os.path.dirname(target)
    temporary = tempfile.mkdtemp(prefix=f".codex-rsync-wt-{os.path.basename(target)}-", dir=parent)
    try:
        git(source, "worktree", "add", "--detach", "--no-checkout", temporary, "HEAD")
        temporary_dot_git = os.path.join(temporary, ".git")
        contents = Path(temporary_dot_git).read_text(encoding="utf-8").strip()
        if not contents.startswith("gitdir:"):
            raise RuntimeError(f"invalid temporary worktree pointer: {temporary_dot_git}")
        git_dir = contents.split(":", 1)[1].strip()
        if not os.path.isabs(git_dir):
            git_dir = os.path.abspath(os.path.join(temporary, git_dir))
        Path(target_dot_git).write_text(f"gitdir: {git_dir}\n", encoding="utf-8")
        Path(os.path.join(git_dir, "gitdir")).write_text(f"{target_dot_git}\n", encoding="utf-8")
    finally:
        shutil.rmtree(temporary, ignore_errors=True)
    return True


def sync_git_metadata(source: str, target: str) -> bool:
    if not ensure_git_link(source, target):
        return False
    source_head = git(source, "rev-parse", "HEAD").stdout.strip().decode("ascii")
    target_head = git(target, "rev-parse", "HEAD").stdout.strip().decode("ascii")
    if source_head != target_head:
        git(target, "update-ref", "--no-deref", "HEAD", source_head)

    source_index = git_path(source, "index")
    target_index = git_path(target, "index")
    if os.path.realpath(source_index) != os.path.realpath(target_index):
        target_git_dir = os.path.dirname(target_index)
        os.makedirs(target_git_dir, exist_ok=True)
        fd, temporary_index = tempfile.mkstemp(prefix="codex-index-sync-", dir=target_git_dir)
        os.close(fd)
        try:
            shutil.copy2(source_index, temporary_index)
            os.replace(temporary_index, target_index)
        finally:
            if os.path.exists(temporary_index):
                os.unlink(temporary_index)
    return True


def ignored_paths(root: str) -> list[bytes]:
    result = git(root, "ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z")
    return [path for path in result.stdout.split(b"\0") if path]


def escape_filter_path(path: bytes) -> bytes:
    if b"\n" in path or b"\r" in path:
        raise RuntimeError("Git-ignored paths containing newlines cannot be represented in rsync filters")
    escaped = bytearray()
    for char in path:
        if char in b"\\*?[] " or char == 9:
            escaped.append(ord("\\"))
        escaped.append(char)
    return bytes(escaped)


def append_filter_rules(rules: bytearray, kind: bytes, paths: list[bytes]) -> None:
    for raw in paths:
        is_directory = raw.endswith(b"/")
        path = raw.rstrip(b"/")
        if not path:
            continue
        rules.extend(kind + b" /" + escape_filter_path(path))
        if is_directory:
            rules.extend(b"/***")
        rules.extend(b"\n")


def tracked_ignore_files(source: str) -> list[bytes]:
    tracked = git(source, "ls-files", "-z").stdout.split(b"\0")
    return [path for path in tracked if path and path.rsplit(b"/", 1)[-1] == b".gitignore"]


def copy_ignore_files(
    rsync: str, source: str, target: str, temp_dir: str, reflink: bool, excludes: list[str]
) -> None:
    paths = tracked_ignore_files(source)
    if not paths:
        return
    list_path = os.path.join(temp_dir, "ignore-files.list")
    Path(list_path).write_bytes(b"\0".join(paths) + b"\0")
    command = [rsync, "-a", "--from0", "--files-from", list_path, "--relative"]
    if reflink:
        command.append("--reflink=auto")
    for pattern in excludes:
        command.extend(["--exclude", pattern])
    command.extend(["--exclude=.git", f"{source.rstrip(os.sep)}/", target])
    run_rsync(command)


def source_symlink_targets(
    source: str, target: str, ignored_paths: list[bytes]
) -> list[bytes]:
    protected: set[bytes] = set()
    target_real = os.path.realpath(target)
    ignored_directories = {
        path.rstrip(b"/").replace(b"/", os.fsencode(os.sep))
        for path in ignored_paths
        if path.endswith(b"/")
    }
    for root, dirs, files in os.walk(source, followlinks=False):
        relative_root = os.path.relpath(root, source)
        relative_root_bytes = b"" if relative_root == "." else os.fsencode(relative_root)
        for name in dirs + files:
            path = os.path.join(root, name)
            if not os.path.islink(path):
                continue
            resolved = os.path.abspath(os.path.join(os.path.dirname(path), os.readlink(path)))
            if os.path.isabs(os.readlink(path)):
                resolved = os.path.abspath(os.readlink(path))
            if resolved == target_real or resolved.startswith(target_real + os.sep):
                relative = os.path.relpath(resolved, target_real).replace(os.sep, "/")
                if os.path.isdir(resolved) and not os.path.islink(resolved):
                    relative += "/"
                protected.add(os.fsencode(relative))
        dirs[:] = [
            name
            for name in dirs
            if name != ".git"
            and os.path.join(relative_root_bytes, os.fsencode(name)) not in ignored_directories
        ]
    return sorted(protected)


def run_rsync(command: list[str]) -> None:
    result = subprocess.run(command, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"rsync exited with code {result.returncode}")


def temporary_root() -> str:
    if os.environ.get("TMPDIR"):
        requested = os.environ["TMPDIR"]
    elif os.environ.get("PREFIX", "").startswith("/data/data/"):
        requested = os.path.join(os.environ["PREFIX"], "tmp")
    else:
        requested = "/var/tmp"
    if not os.path.isdir(requested):
        raise RuntimeError(f"temporary directory not found: {requested}")
    return os.path.realpath(requested)


def operation_lock_root() -> str:
    prefix = os.environ.get("PREFIX", "")
    if prefix.startswith("/data/data/"):
        return os.path.realpath(os.path.join(prefix, "tmp"))
    if os.path.isdir("/var/tmp"):
        return os.path.realpath("/var/tmp")
    return os.path.realpath(tempfile.gettempdir())


def acquire_sync_lock(source: str, target: str):
    # Match the existing cpto lock identity so manual and scripted syncs cannot race.
    identity = "\0".join((os.path.realpath(source), os.path.realpath(target)))
    digest = hashlib.sha256(identity.encode()).hexdigest()[:24]
    lock_path = os.path.join(operation_lock_root(), f"cpto-lock-{digest}.lock")
    lock_file = open(lock_path, "a+")
    try:
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as error:
        lock_file.close()
        raise RuntimeError(f"source/build sync already running for {source} -> {target}") from error
    return lock_file


def sync(source: str, target: str, excludes: list[str]) -> None:
    source = os.path.abspath(source)
    target = os.path.abspath(target)
    if not os.path.isdir(source) or not os.path.isdir(target):
        raise RuntimeError("source and target must both be existing directories")
    if os.path.realpath(source) == os.path.realpath(target):
        raise RuntimeError("source and target must be different directories")
    source_real = os.path.realpath(source)
    target_real = os.path.realpath(target)
    if target_real.startswith(source_real + os.sep) or source_real.startswith(target_real + os.sep):
        raise RuntimeError("build tree must be outside the source checkout")

    rsync = os.environ.get("CODEX_RSYNC") or shutil.which("rsync")
    if not rsync:
        raise RuntimeError("rsync is required to synchronize source and build trees")
    probe = subprocess.run([rsync, "--help"], capture_output=True, text=True, check=False)
    reflink = probe.returncode == 0 and "--reflink=MODE" in probe.stdout
    if not reflink:
        raise RuntimeError("rsync must support --reflink=auto; select a reflink-enabled binary with CODEX_RSYNC")

    temp_root = temporary_root()
    lock_file = acquire_sync_lock(source, target)
    try:
        sync_locked(source, target, excludes, rsync, reflink, temp_root)
    finally:
        lock_file.close()


def sync_locked(source: str, target: str, excludes: list[str], rsync: str, reflink: bool, temp_root: str) -> None:
    source_is_git = git(source, "rev-parse", "--is-inside-work-tree", check=False).returncode == 0
    if not source_is_git:
        raise RuntimeError(f"source is not a Git worktree: {source}")
    linked_worktree = sync_git_metadata(source, target)
    with tempfile.TemporaryDirectory(prefix="codex-rsync-sync-", dir=temp_root) as temp_dir:
        copy_ignore_files(rsync, source, target, temp_dir, reflink, excludes)

        # Git emits ignored directories as single entries unless tracked or
        # non-ignored descendants require the directory to be traversed.
        # This avoids scanning huge generated trees to construct the filter.
        rules = bytearray()
        rules.extend(b"H /.git\nP /.git\n")
        source_ignored = ignored_paths(source)
        target_is_git = git(target, "rev-parse", "--is-inside-work-tree", check=False).returncode == 0
        target_ignored = ignored_paths(target) if target_is_git else []
        append_filter_rules(rules, b"H", source_ignored)
        append_filter_rules(rules, b"P", target_ignored)
        symlink_protected = source_symlink_targets(source, target, source_ignored)
        append_filter_rules(rules, b"P", symlink_protected)
        filter_path = os.path.join(temp_dir, "git-ignore.rules")
        Path(filter_path).write_bytes(rules)

        command = [rsync, "-a", "--delete", "--exclude=.git", "--filter", f"merge {filter_path}"]
        for pattern in excludes:
            command.extend(["--exclude", pattern])
        command.append("--reflink=auto")
        command.extend([f"{source.rstrip(os.sep)}/", target])
        run_rsync(command)

        # Keep ignored source artifacts current when they are newer, without
        # exposing them to --delete.
        if source_ignored:
            ignored_list = os.path.join(temp_dir, "ignored-source.list")
            Path(ignored_list).write_bytes(b"\0".join(source_ignored) + b"\0")
            command = [rsync, "-a", "--update", "--from0", "--files-from", ignored_list, "--recursive"]
            command.append("--reflink=auto")
            for pattern in excludes:
                command.extend(["--exclude", pattern])
            command.extend(["--exclude=.git", f"{source.rstrip(os.sep)}/", target])
            run_rsync(command)

    if linked_worktree:
        sync_git_metadata(source, target)
        mark_skip_worktree(target, symlink_protected)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source")
    parser.add_argument("build_tree")
    parser.add_argument("--exclude", action="append", default=[], help="rsync path pattern to preserve")
    args = parser.parse_args()
    try:
        sync(args.source, args.build_tree, args.exclude)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"source/build sync failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
