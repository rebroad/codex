#!/usr/bin/env python3
"""Assemble one npm release from the newest native payload per target.

Upstream and fork CI jobs publish native payload artifacts independently. This
script combines those payloads into one vendor tree and invokes the shared
upstream-derived npm packager exactly once, so all platform aliases point at
the same immutable release candidate.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
STAGE_SCRIPT = ROOT / "scripts" / "stage_npm_packages.py"
PLATFORM_TARGETS = {
    "linux-x64": "x86_64-unknown-linux-musl",
    "linux-arm64": "aarch64-unknown-linux-musl",
    "darwin-x64": "x86_64-apple-darwin",
    "darwin-arm64": "aarch64-apple-darwin",
    "win32-x64": "x86_64-pc-windows-msvc",
    "win32-arm64": "aarch64-pc-windows-msvc",
    "linux-armv7": "armv7-unknown-linux-gnueabihf",
    "android-arm64": "aarch64-linux-android",
}
TIMESTAMP_RE = re.compile(r"\.(\d{12})(?:\.|$|-)")


def load_stage_module():
    spec = importlib.util.spec_from_file_location("stage_npm_packages", STAGE_SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to load {STAGE_SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-version", required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument(
        "--fork-artifact-dir",
        type=Path,
        required=True,
        help="Directory containing fork-produced codex-npm-*.tgz archives.",
    )
    parser.add_argument(
        "--upstream-version",
        required=True,
        help="Upstream Cargo version whose successful rust-release run supplies desktop targets.",
    )
    parser.add_argument("--upstream-repo", default="openai/codex")
    parser.add_argument("--upstream-workflow-url")
    parser.add_argument(
        "--upstream-vendor-src",
        type=Path,
        help="Pre-extracted upstream vendor tree, primarily useful for local testing.",
    )
    parser.add_argument(
        "--upstream-artifacts-dir",
        type=Path,
        help="Persistent cache for downloaded upstream workflow artifacts.",
    )
    return parser.parse_args()


def resolve_upstream_workflow(repo: str, version: str) -> dict[str, str]:
    output = subprocess.check_output(
        [
            "gh",
            "run",
            "list",
            "--repo",
            repo,
            "--workflow",
            "rust-release.yml",
            "--branch",
            f"rust-v{version}",
            "--limit",
            "20",
            "--json",
            "databaseId,headSha,status,conclusion,url,createdAt",
            "--jq",
            "[.[] | select(.status == \"completed\" and .conclusion == \"success\")] | first",
        ],
        text=True,
    )
    workflow = json.loads(output or "null")
    if not workflow:
        raise RuntimeError(f"No successful rust-release workflow found for {repo} {version}")
    return workflow


def copy_vendor_tree(source: Path, destination: Path) -> None:
    source = source.resolve()
    if not source.is_dir():
        raise RuntimeError(f"Vendor source does not exist: {source}")
    for target_dir in source.iterdir():
        if not target_dir.is_dir() or target_dir.name not in PLATFORM_TARGETS.values():
            continue
        target_destination = destination / target_dir.name
        if target_destination.exists():
            shutil.rmtree(target_destination)
        shutil.copytree(target_dir, target_destination)


def archive_sort_key(package_version: str, archive: Path) -> tuple[str, int, str]:
    timestamp = TIMESTAMP_RE.search(package_version)
    return (
        timestamp.group(1) if timestamp else "",
        archive.stat().st_mtime_ns,
        package_version,
    )


def select_fork_archives(artifact_dir: Path) -> dict[str, tuple[Path, str]]:
    selected: dict[str, tuple[Path, str]] = {}
    for archive in sorted(artifact_dir.glob("codex-npm-*.tgz")):
        with tarfile.open(archive, "r:gz") as package_archive:
            metadata = json.loads(package_archive.extractfile("package/package.json").read())
        package_version = metadata.get("version", "")
        for platform, target in PLATFORM_TARGETS.items():
            if package_version.endswith(f"-{platform}"):
                previous = selected.get(target)
                if previous is None or archive_sort_key(package_version, archive) > archive_sort_key(
                    previous[1], previous[0]
                ):
                    selected[target] = (archive, package_version)
                break
    return selected


def extract_fork_payloads(artifact_dir: Path, vendor_root: Path) -> dict[str, dict[str, str]]:
    selected = select_fork_archives(artifact_dir)
    manifest: dict[str, dict[str, str]] = {}
    for target, (archive, package_version) in selected.items():
        with tempfile.TemporaryDirectory(prefix="codex-fork-package-") as staging:
            staging_path = Path(staging)
            with tarfile.open(archive, "r:gz") as package_archive:
                package_archive.extractall(staging_path, filter="data")
            source = staging_path / "package" / "vendor" / target
            if not source.is_dir():
                raise RuntimeError(f"{archive} does not contain vendor/{target}")
            destination = vendor_root / target
            if destination.exists():
                shutil.rmtree(destination)
            shutil.copytree(source, destination)
        manifest[target] = {"archive": archive.name, "version": package_version}
    return manifest


def main() -> int:
    args = parse_args()
    stage_module = load_stage_module()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    upstream_cache = args.upstream_artifacts_dir or args.output_dir.parent / "upstream-artifacts"

    with tempfile.TemporaryDirectory(prefix="codex-npm-vendor-") as vendor_temp:
        vendor_root = Path(vendor_temp)
        source_manifest: dict[str, dict[str, str]] = {}

        if args.upstream_vendor_src:
            copy_vendor_tree(args.upstream_vendor_src, vendor_root)
            for platform, target in PLATFORM_TARGETS.items():
                if (vendor_root / target).is_dir():
                    source_manifest[target] = {"source": "upstream-vendor-src", "platform": platform}
        else:
            workflow = (
                {"url": args.upstream_workflow_url}
                if args.upstream_workflow_url
                else resolve_upstream_workflow(args.upstream_repo, args.upstream_version)
            )
            stage_module.GITHUB_REPO = args.upstream_repo
            workflow_id = workflow["url"].rstrip("/").split("/")[-1]
            stage_module.install_from_workflow_artifacts(
                workflow_id,
                upstream_cache,
                [stage_module.CODEX_PACKAGE_COMPONENT],
                vendor_root,
            )
            for platform, target in PLATFORM_TARGETS.items():
                if (vendor_root / target).is_dir():
                    source_manifest[target] = {
                        "source": args.upstream_repo,
                        "workflow": workflow["url"],
                        "platform": platform,
                    }

        source_manifest.update(extract_fork_payloads(args.fork_artifact_dir, vendor_root))
        missing = [
            platform
            for platform, target in PLATFORM_TARGETS.items()
            if not (vendor_root / target).is_dir()
        ]
        if missing:
            raise RuntimeError("Missing native payloads for: " + ", ".join(missing))

        subprocess.run(
            [
                sys.executable,
                str(STAGE_SCRIPT),
                "--release-version",
                args.release_version,
                "--package",
                "codex",
                "--vendor-src",
                str(vendor_root),
                "--output-dir",
                str(args.output_dir),
            ],
            check=True,
        )

    manifest_path = args.output_dir / "npm-artifact-sources.json"
    manifest_path.write_text(
        json.dumps(
            {
                "release_version": args.release_version,
                "upstream_version": args.upstream_version,
                "platforms": source_manifest,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    print(f"Wrote artifact source manifest to {manifest_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
