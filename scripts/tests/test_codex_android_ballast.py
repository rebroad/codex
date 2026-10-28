"""Tests for the standalone Android RAM ballast controller."""

from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from codex_android_ballast import MIB, chunk_action


class BallastTest(unittest.TestCase):
    def test_release_to_restore_available(self):
        self.assertEqual(chunk_action(400*MIB, 440*MIB, 20, 32, 100*MIB, 8*MIB, 16*MIB, True), -4)

    def test_release_at_floor(self):
        self.assertEqual(chunk_action(105*MIB, 440*MIB, 20, 32, 100*MIB, 8*MIB, 16*MIB, True), -20)

    def test_hysteresis(self):
        self.assertEqual(chunk_action(435*MIB, 440*MIB, 20, 32, 100*MIB, 8*MIB, 16*MIB, True), 0)

    def test_refill_only_when_cooldown_over_and_enough_headroom(self):
        self.assertEqual(chunk_action(480*MIB, 440*MIB, 20, 32, 100*MIB, 8*MIB, 16*MIB, False), 0)
        self.assertEqual(chunk_action(480*MIB, 440*MIB, 20, 32, 100*MIB, 8*MIB, 16*MIB, True), 1)

    def test_no_refill_at_capacity(self):
        self.assertEqual(chunk_action(480*MIB, 440*MIB, 32, 32, 100*MIB, 8*MIB, 16*MIB, True), 0)

    def test_signal_and_shutdown(self):
        with tempfile.TemporaryDirectory() as d:
            ready = Path(d, "ready")
            status = Path(d, "status")
            proc = subprocess.Popen([
                sys.executable, str(Path(__file__).resolve().parents[1] / "codex_android_ballast.py"),
                "--ready-file", str(ready), "--status-file", str(status),
                "--max-mb", "16", "--chunk-mb", "8", "--ramp-ms", "25",
                "--interval-ms", "25", "--pressure-release-mb", "8",
            ], stderr=subprocess.PIPE, text=True)
            try:
                for _ in range(150):
                    if ready.exists():
                        break
                    self.assertIsNone(proc.poll(), "controller exited early")
                    time.sleep(0.02)
                self.assertTrue(ready.exists())
                if int(status.read_text()) == 0:
                    self.skipTest("Insufficient test host memory for a ballast")
                self.assertEqual(int(status.read_text()), 16)
                proc.send_signal(signal.SIGUSR1)
                for _ in range(150):
                    if status.exists() and int(status.read_text()) < 16:
                        break
                    time.sleep(0.02)
                self.assertEqual(int(status.read_text()), 8)
                proc.send_signal(signal.SIGUSR1)
                time.sleep(0.1)
                self.assertEqual(int(status.read_text()), 8)
            finally:
                proc.terminate()
                proc.communicate(timeout=5)
            self.assertFalse(ready.exists())
            self.assertFalse(status.exists())


if __name__ == "__main__":
    unittest.main()
