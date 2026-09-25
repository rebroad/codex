#!/usr/bin/env python3
"""Resolve the source checkout associated with a justfile invocation."""

import os
import subprocess
import sys
from pathlib import Path


def is_repository(path: Path) -> bool:
    return (path / "justfile").is_file() and (path / "codex-rs").is_dir()


def mirrored_source_for_build_tree(build_root: Path) -> Path | None:
    parts = list(build_root.parts)
    if "builds" not in parts:
        return None

    builds_index = parts.index("builds")
    parts[builds_index] = "@home"
    if parts[-1].endswith(".build"):
        parts[-1] = parts[-1][:-len(".build")]
    elif parts[-1].endswith(".make"):
        parts[-1] = parts[-1][:-len(".make")]
    elif parts[-2].endswith(".build"):
        parts[-2] = parts[-2][:-len(".build")]
        parts.pop()
    elif parts[-2].endswith(".make"):
        parts[-2] = parts[-2][:-len(".make")]
        parts.pop()
    else:
        return None

    source_root = Path(*parts)
    return source_root if is_repository(source_root) else None


def source_for_worktree(build_root: Path) -> Path | None:
    git_path = build_root / ".git"
    if not git_path.is_file():
        return None

    result = subprocess.run(
        ["git", "-C", str(build_root), "rev-parse", "--path-format=absolute", "--git-common-dir"],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return None

    source_root = Path(result.stdout.strip()).resolve().parent
    return source_root if source_root != build_root and is_repository(source_root) else None


def is_build_repo_inside_source(source_root: Path, build_root: Path) -> bool:
    source_root = source_root.resolve()
    build_root = build_root.resolve()
    if build_root == source_root:
        return True

    source_path = os.path.normcase(str(source_root))
    build_path = os.path.normcase(str(build_root))
    is_nested = build_path.startswith(os.path.join(source_path, ""))
    if not is_nested:
        return False

    return True


def is_build_tree_root(repo_root: Path) -> bool:
    return repo_root.name.endswith((".build", ".make")) or repo_root.parent.name.endswith(
        (".build", ".make")
    )


def resolve_source_repo(repo_root: Path) -> Path:
    repo_root = repo_root.resolve()
    override = os.environ.get("CODEX_SOURCE_REPO")
    if override:
        source_root = Path(override).resolve()
        if not is_repository(source_root):
            raise ValueError(f"CODEX_SOURCE_REPO is not a Codex source checkout: {source_root}")
        return source_root

    if is_build_tree_root(repo_root):
        source_root = source_for_worktree(repo_root) or mirrored_source_for_build_tree(repo_root)
        if source_root is None:
            raise ValueError(
                "cannot find the source checkout for this .build/.make tree; "
                "set CODEX_SOURCE_REPO to its path"
            )
        return source_root

    if (repo_root / ".git").exists():
        return repo_root

    fallback = repo_root.parent / "codex"
    return fallback if is_repository(fallback) else repo_root


def main() -> int:
    try:
        if len(sys.argv) == 4 and sys.argv[1] == "--is-build-repo-inside-source":
            print(
                str(
                    is_build_repo_inside_source(Path(sys.argv[2]), Path(sys.argv[3]))
                ).lower()
            )
            return 0
        print(resolve_source_repo(Path(sys.argv[1])))
    except (IndexError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
