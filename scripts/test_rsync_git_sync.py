#!/usr/bin/env python3
"""Integration coverage for the Git-aware source/build rsync helper."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import rsync_git_sync


class RsyncGitSyncTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(
            tempfile.mkdtemp(prefix="codex-rsync-sync-test-", dir=rsync_git_sync.temporary_root())
        )
        self.source = self.root / "source"
        self.target = self.root / "source.build"
        self.source.mkdir()
        self.target.mkdir()

    def tearDown(self) -> None:
        shutil.rmtree(self.root, ignore_errors=True)

    def git(self, *args: str) -> None:
        subprocess.run(["git", "-C", str(self.source), *args], check=True, capture_output=True)

    def test_concurrent_sync_for_same_pair_is_rejected(self) -> None:
        lock = rsync_git_sync.acquire_sync_lock(str(self.source), str(self.target))
        try:
            with self.assertRaisesRegex(RuntimeError, "sync already running"):
                rsync_git_sync.acquire_sync_lock(str(self.source), str(self.target))
        finally:
            lock.close()

    def test_rsync_without_reflink_support_is_rejected(self) -> None:
        fake_rsync = self.root / "rsync-without-reflink"
        fake_rsync.write_text("#!/bin/sh\nprintf 'rsync version 3.2.7\\n'\n")
        fake_rsync.chmod(0o755)
        with patch.dict(os.environ, {"CODEX_RSYNC": str(fake_rsync)}):
            with self.assertRaisesRegex(RuntimeError, "must support --reflink=auto"):
                rsync_git_sync.sync(str(self.source), str(self.target), [])

    def test_independent_target_checkout_is_rejected(self) -> None:
        self.git("init", "-q")
        subprocess.run(["git", "-C", str(self.target), "init", "-q"], check=True)
        with self.assertRaisesRegex(RuntimeError, "independent Git worktree"):
            rsync_git_sync.ensure_git_link(str(self.source), str(self.target))
        self.assertTrue((self.target / ".git").is_dir())

    def test_sync_preserves_ignored_build_outputs_and_applies_source_changes(self) -> None:
        self.git("init", "-q")
        self.git("config", "user.name", "Codex sync test")
        self.git("config", "user.email", "codex-sync-test@example.invalid")
        (self.source / ".gitignore").write_text("ignored-out/\ncache/\npartial/*.tmp\nshared-cache/*.bin\n")
        (self.source / "tracked.txt").write_text("v1\n")
        (self.source / "gone.txt").write_text("remove\n")
        (self.source / "partial").mkdir()
        (self.source / "partial/keep.txt").write_text("kept v1\n")
        (self.source / "shared-cache").mkdir()
        (self.source / "shared-cache/tracked-output.txt").write_text("tracked generated path\n")
        self.git(
            "add", ".gitignore", "tracked.txt", "gone.txt", "partial/keep.txt",
            "shared-cache/tracked-output.txt",
        )
        self.git("commit", "-qm", "fixture")
        self.git("rm", "-q", "gone.txt")
        self.git("commit", "-qm", "remove tracked file")

        (self.source / "tracked.txt").write_text("v2\n")
        (self.source / "partial/keep.txt").write_text("kept v2\n")
        (self.source / "cache").mkdir()
        (self.source / "cache/from-source.bin").write_text("new ignored source\n")
        (self.source / "partial/new.tmp").write_text("ignored source child\n")
        (self.source / "build-cache-alias").symlink_to(self.target / "shared-cache", target_is_directory=True)

        (self.target / "tracked.txt").write_text("old target\n")
        (self.target / "gone.txt").write_text("stale tracked file\n")
        (self.target / "stale-unignored.txt").write_text("remove\n")
        (self.target / "ignored-out").mkdir()
        (self.target / "ignored-out/cache.bin").write_text("preserve\n")
        (self.target / "shared-cache").mkdir()
        (self.target / "shared-cache/output.bin").write_text("preserve symlink target\n")
        (self.target / "shared-cache/tracked-output.txt").write_text("stale generated path\n")
        (self.target / "partial").mkdir()
        (self.target / "partial/keep.txt").write_text("old tracked child\n")
        (self.target / "partial/target.tmp").write_text("preserve ignored child\n")

        rsync = os.environ.get("CODEX_RSYNC") or shutil.which("rsync")
        self.assertIsNotNone(rsync)
        env = dict(os.environ, CODEX_RSYNC=rsync)
        subprocess.run(
            ["python3", str(Path(__file__).with_name("rsync_git_sync.py")), str(self.source), str(self.target)],
            check=True,
            env=env,
        )

        self.assertEqual((self.target / "tracked.txt").read_text(), "v2\n")
        self.assertFalse((self.target / "gone.txt").exists())
        self.assertFalse((self.target / "stale-unignored.txt").exists())
        self.assertTrue((self.target / "ignored-out/cache.bin").exists())
        self.assertTrue((self.target / "shared-cache/output.bin").exists())
        self.assertEqual((self.target / "partial/keep.txt").read_text(), "kept v2\n")
        self.assertTrue((self.target / "partial/target.tmp").exists())
        self.assertEqual((self.target / "cache/from-source.bin").read_text(), "new ignored source\n")
        self.assertEqual(rsync_git_sync.same_common_git_dir(str(self.source), str(self.target)), True)
        skip_flag = subprocess.run(
            ["git", "-C", str(self.target), "ls-files", "-v", "--", "shared-cache/tracked-output.txt"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        self.assertTrue(skip_flag.startswith("S "), skip_flag)


if __name__ == "__main__":
    unittest.main()
