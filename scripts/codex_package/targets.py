"""Supported package targets and default binary discovery."""

import os
import platform
import stat
import sys
from dataclasses import dataclass
from pathlib import Path

# Resolve from this module so direct script invocations and inline imports do
# not depend on being launched through `just` or a workflow with a global env.
REPO_ROOT = Path(__file__).resolve().parents[2]
os.environ["CODEX_REPO_ROOT"] = str(REPO_ROOT)


@dataclass(frozen=True)
class TargetSpec:
    target: str
    is_windows: bool
    is_linux: bool
    is_android: bool
    dotslash_platform: str

    @property
    def exe_suffix(self) -> str:
        return ".exe" if self.is_windows else ""

    @property
    def rg_name(self) -> str:
        return f"rg{self.exe_suffix}"


@dataclass(frozen=True)
class PackageVariant:
    name: str
    cargo_bin: str
    executable_stem: str

    def entrypoint_name(self, spec: TargetSpec) -> str:
        return f"{self.executable_stem}{spec.exe_suffix}"


@dataclass(frozen=True)
class PackageInputs:
    entrypoint_bin: Path
    code_mode_host_bin: Path
    rg_bin: Path | None
    bwrap_bin: Path | None
    codex_command_runner_bin: Path | None
    codex_windows_sandbox_setup_bin: Path | None
    android_cxx_runtime_bin: Path | None = None


PACKAGE_VARIANTS: dict[str, PackageVariant] = {
    "codex": PackageVariant(
        name="codex",
        cargo_bin="codex",
        executable_stem="codex",
    ),
    "codex-app-server": PackageVariant(
        name="codex-app-server",
        cargo_bin="codex-app-server",
        executable_stem="codex-app-server",
    ),
}


TARGET_SPECS: dict[str, TargetSpec] = {
    "x86_64-unknown-linux-gnu": TargetSpec(
        target="x86_64-unknown-linux-gnu",
        is_windows=False,
        is_linux=True,
        is_android=False,
        dotslash_platform="linux-x86_64",
    ),
    "x86_64-unknown-linux-musl": TargetSpec(
        target="x86_64-unknown-linux-musl",
        is_windows=False,
        is_linux=True,
        is_android=False,
        dotslash_platform="linux-x86_64",
    ),
    "aarch64-unknown-linux-gnu": TargetSpec(
        target="aarch64-unknown-linux-gnu",
        is_windows=False,
        is_linux=True,
        is_android=False,
        dotslash_platform="linux-aarch64",
    ),
    "aarch64-unknown-linux-musl": TargetSpec(
        target="aarch64-unknown-linux-musl",
        is_windows=False,
        is_linux=True,
        is_android=False,
        dotslash_platform="linux-aarch64",
    ),
    "x86_64-apple-darwin": TargetSpec(
        target="x86_64-apple-darwin",
        is_windows=False,
        is_linux=False,
        is_android=False,
        dotslash_platform="macos-x86_64",
    ),
    "aarch64-apple-darwin": TargetSpec(
        target="aarch64-apple-darwin",
        is_windows=False,
        is_linux=False,
        is_android=False,
        dotslash_platform="macos-aarch64",
    ),
    "x86_64-pc-windows-msvc": TargetSpec(
        target="x86_64-pc-windows-msvc",
        is_windows=True,
        is_linux=False,
        is_android=False,
        dotslash_platform="windows-x86_64",
    ),
    "aarch64-pc-windows-msvc": TargetSpec(
        target="aarch64-pc-windows-msvc",
        is_windows=True,
        is_linux=False,
        is_android=False,
        dotslash_platform="windows-aarch64",
    ),
    "aarch64-linux-android": TargetSpec(
        target="aarch64-linux-android",
        is_windows=False,
        is_linux=False,
        is_android=True,
        dotslash_platform="android-arm64",
    ),
}


HOST_RELEASE_TARGETS: dict[tuple[str, str], str] = {
    ("darwin", "aarch64"): "aarch64-apple-darwin",
    ("darwin", "x86_64"): "x86_64-apple-darwin",
    ("linux", "aarch64"): "aarch64-unknown-linux-musl",
    ("linux", "x86_64"): "x86_64-unknown-linux-musl",
    ("android", "aarch64"): "aarch64-linux-android",
    ("windows", "aarch64"): "aarch64-pc-windows-msvc",
    ("windows", "x86_64"): "x86_64-pc-windows-msvc",
}


def default_target() -> str:
    system = "android" if sys.platform == "android" else platform.system().lower()
    machine = normalize_machine(platform.machine())
    target = HOST_RELEASE_TARGETS.get((system, machine))
    if target is None:
        supported = ", ".join(sorted(TARGET_SPECS))
        raise RuntimeError(
            f"Unsupported host platform {platform.system()}/{platform.machine()}. "
            f"Pass --target explicitly. Supported targets: {supported}"
        )
    return target


def resolve_input_path(
    explicit_path: Path | None,
    description: str,
    flag_name: str,
) -> Path:
    if explicit_path is not None:
        path = explicit_path.resolve()
        if not path.is_file():
            raise RuntimeError(f"{description} does not exist: {path}")
        if not is_executable(path):
            raise RuntimeError(f"{description} is not executable: {path}")
        return path

    raise RuntimeError(f"Must specify {flag_name} for {description}.")


def is_executable(path: Path) -> bool:
    return bool(path.stat().st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))


def normalize_machine(machine: str) -> str:
    machine = machine.lower()
    if machine in ("amd64", "x86_64"):
        return "x86_64"
    if machine in ("aarch64", "arm64"):
        return "aarch64"
    return machine
