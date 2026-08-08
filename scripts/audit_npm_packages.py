#!/usr/bin/env python3
"""Audit the complete Codex npm artifact set before publication."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import tarfile
import tempfile
from pathlib import Path


PLATFORMS = {
    "linux-x64": {
        "target": "x86_64-unknown-linux-musl",
        "os": "linux",
        "cpu": "x64",
        "file_tokens": ("ELF", "x86-64"),
        "host": True,
    },
    "linux-arm64": {
        "target": "aarch64-unknown-linux-musl",
        "os": "linux",
        "cpu": "arm64",
        "file_tokens": ("ELF", "ARM aarch64"),
        "host": True,
    },
    "darwin-x64": {
        "target": "x86_64-apple-darwin",
        "os": "darwin",
        "cpu": "x64",
        "file_tokens": ("Mach-O", "x86_64"),
        "host": True,
    },
    "darwin-arm64": {
        "target": "aarch64-apple-darwin",
        "os": "darwin",
        "cpu": "arm64",
        "file_tokens": ("Mach-O", "arm64"),
        "host": True,
    },
    "win32-x64": {
        "target": "x86_64-pc-windows-msvc",
        "os": "win32",
        "cpu": "x64",
        "file_tokens": ("PE32+", "x86-64"),
        "host": True,
    },
    "win32-arm64": {
        "target": "aarch64-pc-windows-msvc",
        "os": "win32",
        "cpu": "arm64",
        "file_tokens": ("PE32+", "ARM64"),
        "host": True,
    },
    "linux-armv7": {
        "target": "armv7-unknown-linux-gnueabihf",
        "os": "linux",
        "cpu": "arm",
        "file_tokens": ("ELF", "ARM"),
        "host": True,
    },
    "android-arm64": {
        "target": "aarch64-linux-android",
        "os": "linux",
        "cpu": "arm64",
        "file_tokens": ("ELF", "ARM aarch64"),
        "host": False,
    },
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-dir", type=Path, required=True)
    parser.add_argument("--expected-version", required=True)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_package(archive: Path, destination: Path) -> dict:
    with tarfile.open(archive, "r:gz") as package_archive:
        member = package_archive.getmember("package/package.json")
        package_archive.extractall(destination, filter="data")
    return json.loads((destination / member.name).read_text(encoding="utf-8"))


def classify(version: str) -> str | None:
    for platform in PLATFORMS:
        if version.endswith(f"-{platform}"):
            return platform
    return None


def check_file(path: Path, expected_tokens: tuple[str, ...]) -> str:
    if not path.exists():
        raise RuntimeError(f"Missing native executable: {path}")
    output = subprocess.check_output(["file", "-b", str(path)], text=True).strip()
    if any(token not in output for token in expected_tokens):
        raise RuntimeError(f"Unexpected file type for {path}: {output}")
    return output


def main() -> int:
    args = parse_args()
    artifact_dir = args.artifact_dir.resolve()
    archives = sorted(artifact_dir.glob("codex-npm-*.tgz"))
    if len(archives) != len(PLATFORMS) + 1:
        raise RuntimeError(f"Expected 9 npm archives, found {len(archives)}")

    root_archive: Path | None = None
    platform_archives: dict[str, Path] = {}
    package_versions: dict[str, str] = {}
    for archive in archives:
        checksum_file = archive.with_name(archive.name + ".sha256")
        if not checksum_file.exists():
            raise RuntimeError(f"Missing checksum file: {checksum_file}")
        expected_hash = checksum_file.read_text(encoding="utf-8").split()[0]
        if sha256(archive) != expected_hash:
            raise RuntimeError(f"Checksum mismatch: {archive}")

        with tempfile.TemporaryDirectory(prefix="codex-npm-audit-") as staging:
            metadata = read_package(archive, Path(staging))
        if metadata.get("name") != "@reb.ai/codex":
            raise RuntimeError(f"Unexpected package name in {archive}: {metadata.get('name')}")
        version = metadata.get("version")
        if not isinstance(version, str):
            raise RuntimeError(f"Missing package version in {archive}")
        platform = classify(version)
        if platform is None:
            if root_archive is not None:
                raise RuntimeError("Duplicate root npm archive")
            root_archive = archive
            package_versions["root"] = version
        else:
            if platform in platform_archives:
                raise RuntimeError(f"Duplicate platform archive: {platform}")
            platform_archives[platform] = archive
            package_versions[platform] = version

    if root_archive is None or set(platform_archives) != set(PLATFORMS):
        missing = sorted(set(PLATFORMS) - set(platform_archives))
        raise RuntimeError(f"Missing root or platform archives; missing: {missing}")
    if package_versions["root"] != args.expected_version:
        raise RuntimeError(
            f"Root version {package_versions['root']} does not match {args.expected_version}"
        )
    for platform in PLATFORMS:
        expected = f"{args.expected_version}-{platform}"
        if package_versions[platform] != expected:
            raise RuntimeError(
                f"{platform} version {package_versions[platform]} does not match {expected}"
            )

    with tempfile.TemporaryDirectory(prefix="codex-npm-audit-root-") as root_staging:
        root_metadata = read_package(root_archive, Path(root_staging))
        wrapper = Path(root_staging) / "package" / "bin" / "codex.js"
        if not wrapper.is_file():
            raise RuntimeError("Root package is missing bin/codex.js")
        wrapper_text = wrapper.read_text(encoding="utf-8")
        for target in (entry["target"] for entry in PLATFORMS.values()):
            if target not in wrapper_text:
                raise RuntimeError(f"Root wrapper does not support target {target}")
        optional_dependencies = root_metadata.get("optionalDependencies", {})
        for platform in PLATFORMS:
            dependency_name = f"@reb.ai/codex-{platform}"
            expected = f"npm:@reb.ai/codex@{args.expected_version}-{platform}"
            if optional_dependencies.get(dependency_name) != expected:
                raise RuntimeError(
                    f"Root alias {dependency_name} is {optional_dependencies.get(dependency_name)}, expected {expected}"
                )

    for platform, config in PLATFORMS.items():
        with tempfile.TemporaryDirectory(prefix="codex-npm-audit-platform-") as staging:
            metadata = read_package(platform_archives[platform], Path(staging))
            package_root = Path(staging) / "package"
            if metadata.get("os") != [config["os"]] or metadata.get("cpu") != [config["cpu"]]:
                raise RuntimeError(f"Incorrect os/cpu metadata for {platform}")
            vendor = package_root / "vendor" / config["target"]
            binary = vendor / "bin" / ("codex.exe" if config["os"] == "win32" else "codex")
            file_description = check_file(binary, config["file_tokens"])
            print(f"{platform}: {file_description}")
            if config["host"]:
                host = vendor / "bin" / (
                    "codex-code-mode-host.exe" if config["os"] == "win32" else "codex-code-mode-host"
                )
                check_file(host, config["file_tokens"])
            if platform == "android-arm64":
                runtime = vendor / "bin" / "libc++_shared.so"
                if not runtime.is_file():
                    raise RuntimeError("Android package is missing libc++_shared.so")
            if platform in {"linux-x64", "linux-arm64"}:
                bwrap = vendor / "codex-resources" / "bwrap"
                if not bwrap.is_file():
                    raise RuntimeError(f"{platform} package is missing bundled bwrap")
            strings = subprocess.run(
                ["strings", str(binary)], capture_output=True, text=True, check=True
            ).stdout.lower()
            if "mcp" not in strings:
                raise RuntimeError(f"{platform} binary does not contain MCP support markers")

    print(f"Audited {len(archives)} Codex npm archives for {args.expected_version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
