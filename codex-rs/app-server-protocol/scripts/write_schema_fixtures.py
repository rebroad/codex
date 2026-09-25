#!/usr/bin/env python3

import argparse
import os
from pathlib import Path
import subprocess


def sibling_build_target_dir(workspace_root: Path) -> Path | None:
    """Return the sibling build target directory when this is a source checkout."""
    repo_root = workspace_root.parent
    if repo_root.name.endswith((".build", ".make")):
        return None

    for suffix in (".build", ".make"):
        build_root = Path(f"{repo_root}{suffix}")
        if build_root.is_dir():
            return build_root / "codex-rs" / "target"
    return None


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Regenerate vendored app-server schema fixtures"
    )
    parser.add_argument(
        "--schema-root",
        type=Path,
        help="root directory containing the schema fixtures",
    )
    parser.add_argument(
        "-p",
        "--prettier",
        type=Path,
        help="optional Prettier executable used to format TypeScript files",
    )
    parser.add_argument(
        "--experimental",
        action="store_true",
        help="regenerate the precomputed experimental exports",
    )
    args = parser.parse_args()

    workspace_root = Path(__file__).resolve().parents[2]
    repository_schema_root = workspace_root / "app-server-protocol" / "schema"
    schema_root = args.schema_root or repository_schema_root

    env = os.environ.copy()
    env["CODEX_APP_SERVER_SCHEMA_ROOT"] = str(schema_root)
    env["CODEX_APP_SERVER_SCHEMA_EXPERIMENTAL"] = "1" if args.experimental else "0"
    if args.prettier:
        env["CODEX_APP_SERVER_SCHEMA_PRETTIER"] = str(args.prettier)
    if "CARGO_TARGET_DIR" not in env:
        build_target_dir = sibling_build_target_dir(workspace_root)
        if build_target_dir is not None:
            env["CARGO_TARGET_DIR"] = str(build_target_dir)

    subprocess.run(
        [
            "cargo",
            "test",
            "--locked",
            "-p",
            "codex-app-server-protocol",
            "--lib",
            "schema_fixtures_tests::write_schema_fixtures_from_env",
            "--",
            "--exact",
            "--ignored",
        ],
        cwd=workspace_root,
        env=env,
        check=True,
    )

    # Scratch exports and experimental-only bundles do not update checked-in SDK code.
    if (
        not args.experimental
        and schema_root.resolve() == repository_schema_root.resolve()
    ):
        sdk_root = workspace_root.parent / "sdk" / "python"
        subprocess.run(
            [
                "uv",
                "run",
                "--project",
                str(sdk_root),
                "--frozen",
                "--only-group",
                "test",
                "python",
                str(sdk_root / "scripts" / "update_sdk_artifacts.py"),
                "generate-types",
                "--schema-dir",
                str(schema_root / "json"),
            ],
            cwd=workspace_root,
            check=True,
        )


if __name__ == "__main__":
    main()
