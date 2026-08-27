"""Align build-tree workspace package versions without resolving dependencies."""

import argparse
import hashlib
import json
import os
import re
import subprocess
import tempfile
from pathlib import Path

PACKAGE_BLOCK = re.compile(r"(?ms)^\[\[package\]\]\n.*?(?=^\[\[package\]\]|\Z)")
PACKAGE_NAME = re.compile(r'^name = "([^"]+)"$', re.MULTILINE)
PACKAGE_VERSION = re.compile(r'^version = "([^"]+)"$', re.MULTILINE)


def workspace_versions(manifest: Path) -> dict[str, str]:
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--locked",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            str(manifest),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(result.stdout)
    workspace_members = set(metadata["workspace_members"])
    return {
        package["name"]: package["version"]
        for package in metadata["packages"]
        if package["id"] in workspace_members
    }


def normalize(lock_text: str, versions: dict[str, str]) -> str:
    found: set[str] = set()

    def normalize_block(match: re.Match[str]) -> str:
        block = match.group(0)
        name_match = PACKAGE_NAME.search(block)
        if not name_match or name_match.group(1) not in versions:
            return block
        name = name_match.group(1)
        version = versions[name]
        normalized_block = PACKAGE_VERSION.sub(f'version = "{version}"', block, count=1)
        found.add(name)
        return normalized_block

    normalized_lock = PACKAGE_BLOCK.sub(normalize_block, lock_text)
    missing = set(versions) - found
    if missing:
        raise RuntimeError(
            f"workspace packages missing from Cargo.lock: {sorted(missing)}"
        )
    return normalized_lock


def graph_fingerprint(lock_text: str, versions: dict[str, str]) -> str:
    canonical = normalize(lock_text, versions)

    def remove_workspace_version(match: re.Match[str]) -> str:
        block = match.group(0)
        name_match = PACKAGE_NAME.search(block)
        if name_match and name_match.group(1) in versions:
            block = PACKAGE_VERSION.sub('version = "<workspace>"', block, count=1)
        return block

    canonical = PACKAGE_BLOCK.sub(remove_workspace_version, canonical)
    return hashlib.sha256(canonical.encode()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--source-lock", type=Path, required=True)
    parser.add_argument("--build-lock", type=Path)
    parser.add_argument("--fingerprint", action="store_true")
    args = parser.parse_args()

    versions = workspace_versions(args.manifest)
    source_text = args.source_lock.read_text()
    if args.fingerprint:
        print(graph_fingerprint(source_text, versions))
        return 0

    if args.build_lock is None:
        parser.error("--build-lock is required unless --fingerprint is used")
    build_text = normalize(args.build_lock.read_text(), versions)
    args.build_lock.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        "w", dir=args.build_lock.parent, prefix=".Cargo.lock.", delete=False
    ) as temporary:
        temporary.write(build_text)
        temporary_path = Path(temporary.name)
    os.replace(temporary_path, args.build_lock)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
