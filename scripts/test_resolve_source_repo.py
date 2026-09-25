import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from resolve_source_repo import is_build_repo_inside_source, resolve_source_repo


class ResolveSourceRepoTests(unittest.TestCase):
    def test_rejects_build_directory_nested_in_source(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            source_root = Path(temp_dir) / "codex"
            build_root = source_root / "codex.build"
            build_root.mkdir(parents=True)

            self.assertTrue(is_build_repo_inside_source(source_root, build_root))

    def test_resolves_worktree_under_build_container_to_source(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            source_root = root / "codex"
            source_root.mkdir()
            (source_root / "justfile").touch()
            (source_root / "codex-rs").mkdir()
            subprocess.run(["git", "init", "-q", str(source_root)], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(source_root),
                    "-c",
                    "user.name=Codex Test",
                    "-c",
                    "user.email=codex-test@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "initial",
                ],
                check=True,
                capture_output=True,
            )
            build_root = root / "codex.build" / "build"
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(source_root),
                    "worktree",
                    "add",
                    "--detach",
                    "--quiet",
                    str(build_root),
                ],
                check=True,
                capture_output=True,
            )

            self.assertEqual(resolve_source_repo(build_root), source_root)
            self.assertFalse(is_build_repo_inside_source(source_root, build_root))

    def test_resolves_mirrored_build_tree_to_source_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            build_root = root / "builds" / "rebroad" / "src" / "codex.build"
            source_root = root / "@home" / "rebroad" / "src" / "codex"
            for repo in (build_root, source_root):
                (repo / "codex-rs").mkdir(parents=True)
                (repo / "justfile").touch()

            with patch.dict("os.environ", {}, clear=True):
                self.assertEqual(resolve_source_repo(build_root), source_root)

    def test_keeps_source_checkout_as_source(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            source_root = Path(temp_dir) / "codex"
            (source_root / "codex-rs").mkdir(parents=True)
            (source_root / "justfile").touch()
            (source_root / ".git").mkdir()

            with patch.dict("os.environ", {}, clear=True):
                self.assertEqual(resolve_source_repo(source_root), source_root)

    def test_explicit_source_repo_overrides_build_tree_mapping(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            build_root = root / "codex.build"
            source_root = root / "source"
            for repo in (build_root, source_root):
                (repo / "codex-rs").mkdir(parents=True)
                (repo / "justfile").touch()

            with patch.dict("os.environ", {"CODEX_SOURCE_REPO": str(source_root)}):
                self.assertEqual(resolve_source_repo(build_root), source_root)


if __name__ == "__main__":
    unittest.main()
