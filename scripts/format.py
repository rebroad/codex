#!/usr/bin/env python3
"""Format repository sources or check that they are already formatted."""

import argparse
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def configure_uv_cache() -> None:
    """Choose a writable uv cache when the default home cache is unavailable."""
    if os.environ.get("UV_CACHE_DIR"):
        return

    candidates = []
    if xdg_cache_home := os.environ.get("XDG_CACHE_HOME"):
        candidates.append(Path(xdg_cache_home) / "codex" / "uv")
    candidates.extend(
        [
            Path.home() / ".cache" / "codex" / "uv",
            REPO_ROOT.parent / f"{REPO_ROOT.name}.build" / ".uv-cache",
            Path(tempfile.gettempdir()) / "codex-uv-cache",
        ]
    )

    for candidate in candidates:
        try:
            candidate.mkdir(parents=True, exist_ok=True)
            probe = candidate / ".write-test"
            probe.touch()
            probe.unlink()
        except OSError:
            continue
        os.environ["UV_CACHE_DIR"] = str(candidate)
        return


@dataclass(frozen=True)
class Command:
    args: tuple[str, ...]
    cwd: Path = REPO_ROOT
    env: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True)
class FormatterGroup:
    name: str
    commands: tuple[Command, ...]


@dataclass(frozen=True)
class FormatterResult:
    name: str
    output: str
    returncode: int


def just_formatter_group(*, check: bool) -> FormatterGroup:
    args = ["just", "--unstable", "--fmt"]
    if check:
        args.append("--check")
    return FormatterGroup("Just", (Command(tuple(args)),))


def rust_formatter_group(*, check: bool) -> FormatterGroup:
    if shutil.which("rustup") is not None:
        args = ["cargo", "fmt"]
    else:
        encoded_paths = subprocess.check_output(
            ["git", "ls-files", "-z", "--", "codex-rs"],
            cwd=REPO_ROOT,
        ).split(b"\0")
        rust_files = [
            str(Path(os.fsdecode(path)).relative_to("codex-rs"))
            for path in encoded_paths
            if path.endswith(b".rs")
        ]
        args = ["rustfmt", *rust_files]
    if check:
        args.append("--check")
    command = Command(tuple(args), REPO_ROOT / "codex-rs")
    return FormatterGroup("Rust", (command,))


def buildifier_formatter_group(*, check: bool) -> FormatterGroup:
    repository_files = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=REPO_ROOT,
    ).split(b"\0")
    buildifier_files: list[str] = []
    for encoded_path in repository_files:
        if not encoded_path:
            continue
        path = Path(os.fsdecode(encoded_path))
        name = path.name
        if (
            name in {"BUILD", "WORKSPACE", "MODULE.bazel"}
            or name.startswith(("BUILD.", "WORKSPACE."))
            or name.endswith((".BUILD.bazel", ".MODULE.bazel", ".bzl", ".sky"))
            or ".bzl." in name
            or ".sky." in name
        ):
            buildifier_files.append(path.as_posix())
    buildifier_files.sort()

    buildifier_runner = shutil.which("dotslash")
    if buildifier_runner is not None:
        buildifier_command = [
            buildifier_runner,
            str(REPO_ROOT / "tools" / "buildifier"),
        ]
    else:
        buildifier_runner = shutil.which("buildifier")
        buildifier_command = [buildifier_runner or "dotslash"]
        if buildifier_runner is None:
            buildifier_command.append(str(REPO_ROOT / "tools" / "buildifier"))

    buildifier_args = [
        *buildifier_command,
        "-mode=check" if check else "-mode=fix",
        "-lint=off",
        *buildifier_files,
    ]
    return FormatterGroup("Bazel/Starlark", (Command(tuple(buildifier_args)),))


def formatter_build_tree() -> Path | None:
    return next(
        (
            candidate
            for suffix in (".build", ".make")
            if (candidate := REPO_ROOT.parent / f"{REPO_ROOT.name}{suffix}").is_dir()
        ),
        None,
    )


def ruff_command(project: str, dependency_group: str | None = None) -> list[str]:
    """Use a native Ruff when available, otherwise run the locked uv project."""
    if ruff := shutil.which("ruff"):
        return [ruff]

    command = ["uv", "run", "--frozen", "--project", project]
    if dependency_group is not None:
        command.extend(["--only-group", dependency_group])
    command.append("ruff")
    return command


def python_sdk_formatter_group(*, check: bool) -> FormatterGroup:
    # Each `--project` retains its local dependency and Ruff configuration context.
    build_tree = formatter_build_tree()
    sdk_env = (
        (("UV_PROJECT_ENVIRONMENT", str(build_tree / "sdk-python-venv")),)
        if build_tree is not None
        else ()
    )
    ruff_run_args = ruff_command("sdk/python", "format")
    format_args = [
        *ruff_run_args,
        "format",
    ]
    if check:
        format_args.append("--check")
        # `ruff check --diff` reports lint-driven rewrites without changing files.
        # It is the check-mode counterpart of `--fix --fix-only`, not a full lint gate.
        lint_args = ["check", "--diff"]
    else:
        # Ruff's lint fixer and formatter are separate passes: the first applies
        # fixable lint rewrites, while the second formats source layout.
        lint_args = ["check", "--fix", "--fix-only"]

    return FormatterGroup(
        "Python SDK",
        (
            Command((*ruff_run_args, *lint_args, "sdk/python"), env=sdk_env),
            Command((*format_args, "sdk/python"), env=sdk_env),
        ),
    )


def python_scripts_formatter_group(*, check: bool) -> FormatterGroup:
    # The SDK and internal scripts intentionally use separate project roots so
    # uv and Ruff retain each project's configuration context.
    args = [*ruff_command("scripts"), "format"]
    build_tree = formatter_build_tree()
    scripts_env = (
        (("UV_PROJECT_ENVIRONMENT", str(build_tree / "scripts-venv")),)
        if build_tree is not None
        else ()
    )
    if check:
        args.append("--check")
    args.append("scripts")
    return FormatterGroup("Python scripts", (Command(tuple(args), env=scripts_env),))


def formatter_groups(*, check: bool) -> tuple[FormatterGroup, ...]:
    return (
        just_formatter_group(check=check),
        rust_formatter_group(check=check),
        buildifier_formatter_group(check=check),
        python_sdk_formatter_group(check=check),
        python_scripts_formatter_group(check=check),
    )


def run_formatter_group(group: FormatterGroup) -> FormatterResult:
    """Run one formatter group sequentially and return its buffered output."""
    for command in group.commands:
        try:
            process = subprocess.run(
                command.args,
                cwd=command.cwd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                check=False,
                env={**os.environ, **dict(command.env)},
            )
        except OSError as error:
            output = f"$ {shlex.join(command.args)}\n{error}\n"
            return FormatterResult(group.name, output, 1)

        if process.returncode != 0:
            output = f"$ {shlex.join(command.args)}\n{process.stdout}"
            if process.stdout and not process.stdout.endswith("\n"):
                output += "\n"
            return FormatterResult(group.name, output, process.returncode)

    return FormatterResult(group.name, "", 0)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--check",
        action="store_true",
        help="check formatting without modifying files",
    )
    args = parser.parse_args()
    configure_uv_cache()
    try:
        groups = formatter_groups(check=args.check)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Unable to configure formatters: {error}", file=sys.stderr)
        return 1

    failures: list[str] = []
    try:
        parallelism = max(1, int(os.environ.get("CODEX_FORMAT_PARALLELISM", "1")))
    except ValueError:
        parallelism = 1
    with ThreadPoolExecutor(max_workers=min(parallelism, len(groups))) as executor:
        futures = [executor.submit(run_formatter_group, group) for group in groups]
        for future in as_completed(futures):
            result = future.result()
            if result.returncode != 0:
                failures.append(result.name)
                print(f"==> {result.name} formatter failed", file=sys.stderr)
                print(result.output, end="", file=sys.stderr)

    if failures:
        print(f"Formatting failed: {', '.join(failures)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
