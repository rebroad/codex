"""Tests for the standalone Android RAM ballast controller."""

import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from codex_android_ballast import (
    MIB, atomic_status, chunk_action, read_psi_sample, read_tuning,
    record_tuning, recovery_refill_decision, tune_next_mb,
)


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

    def test_psi_samples_must_be_fresh(self):
        with tempfile.TemporaryDirectory() as d:
            sample = Path(d, "psi")
            sample.write_text("99000000 12000 3000\n", encoding="ascii")
            self.assertEqual(read_psi_sample(sample, now_wall=100),
                             (99000000, 12000, 3000))
            self.assertIsNone(read_psi_sample(sample, now_wall=105))
            sample.write_text("101000000 12000 3000\n", encoding="ascii")
            self.assertIsNone(read_psi_sample(sample, now_wall=100))
            sample.write_text("not a valid sample\n", encoding="ascii")
            self.assertIsNone(read_psi_sample(sample, now_wall=100))

    def test_adaptive_refill_recovers_after_stable_low_psi(self):
        floor = 1300 * MIB
        available = 6000 * MIB
        chunk = 8 * MIB
        quiet = (99_000_000, 0, 0)
        allowed = lambda seconds, sample: recovery_refill_decision(
            available, floor, chunk, 0, 32, 20, seconds, sample)
        self.assertEqual(allowed(10, quiet), (False, "quiet"))
        self.assertEqual(allowed(31, quiet), (True, "safe"))
        self.assertEqual(allowed(59, None), (False, "quiet"))
        self.assertEqual(allowed(61, None), (True, "safe"))
        self.assertEqual(allowed(31, (99_000_000, 12000, 0)),
                         (False, "psi"))
        self.assertEqual(allowed(31, (99_000_000, 0, 3000)),
                         (False, "psi"))
        self.assertEqual(recovery_refill_decision(
            (floor + 256 * MIB), floor, chunk, 0, 32, 20, 120, quiet),
            (False, "headroom"))
        self.assertEqual(recovery_refill_decision(
            available, floor, chunk, 32, 32, 20, 120, quiet),
            (False, "full"))
        self.assertEqual(recovery_refill_decision(
            available, floor, chunk, 0, 32, 4, 120, quiet),
            (False, "cooldown"))

    def test_autotune_step_with_clean_build(self):
        self.assertEqual(tune_next_mb(256, 256, 0, 3, 180, True, 0, 384)[0], 288)
        self.assertEqual(tune_next_mb(256, 256, 240, 0, 180, True, 0, 384)[0], 224)
        self.assertEqual(tune_next_mb(384, 384, 0, 3, 180, True, 0, 384)[0], 384)

    def test_autotune_does_not_chase_oom_restarts(self):
        self.assertEqual(tune_next_mb(256, 256, 0, 10, 180, True, 1000000, 384)[0], 256)
        self.assertEqual(tune_next_mb(256, 256, 0, 10, 180, False, 0, 384)[0], 256)
        self.assertEqual(tune_next_mb(256, 256, 0, 10, 30, True, 0, 384)[0], 256)
        self.assertEqual(tune_next_mb(256, 64, 0, 0, 180, True, 0, 384)[0], 256)

    def test_autotune_history_roundtrip(self):
        with tempfile.TemporaryDirectory() as d:
            history = Path(d, "history.json")
            outcome = Path(d, "outcome")
            outcome.write_text("0 0\n")
            stats = dict(system_ram_mb=4096, max_mb=256, initial_mb=256,
                         lowest_mb=0, pressure_signals=3, released_mb=256,
                         refilled_mb=0, elapsed_s=180)
            record_tuning(history, outcome, stats, 384)
            self.assertEqual(read_tuning(history, 256, 384, 4096), 288)
            self.assertEqual(read_tuning(history, 256, 384, 8192), 256)
            outcome.write_text("0 90000000\n")
            stats["max_mb"] = 288
            stats["initial_mb"] = 288
            record_tuning(history, outcome, stats, 384)
            self.assertEqual(read_tuning(history, 256, 384, 4096), 288)

    def test_autotune_does_not_grow_long_empty_reserve(self):
        with tempfile.TemporaryDirectory() as d:
            history = Path(d, "history.json")
            result = Path(d, "result")
            result.write_text("0 0\n", encoding="ascii")
            stats = dict(system_ram_mb=4096, max_mb=256,
                         initial_mb=256, lowest_mb=0,
                         pressure_signals=5, released_mb=256, refilled_mb=0,
                         elapsed_s=400, zero_held_seconds=350)
            record_tuning(history, result, stats, 384)
            saved = json.loads(history.read_text(encoding="utf-8"))
            self.assertEqual(saved["recommended_mb"], 256)
            self.assertIn("long empty reserve", saved["observations"][-1]["decision"])

    def test_atomic_telemetry_snapshot(self):
        with tempfile.TemporaryDirectory() as d:
            status = Path(d, "status")
            telemetry = Path(d, "telemetry")
            atomic_status(status, [None, None], 8*MIB, telemetry,
                          max_mb=256, released_chunks=12, refilled_chunks=3)
            self.assertEqual(status.read_text(), "16\n")
            self.assertEqual(telemetry.read_text(), "16 256 96 24\n")

    def test_signal_and_shutdown(self):
        with tempfile.TemporaryDirectory() as d:
            ready = Path(d, "ready")
            status = Path(d, "status")
            telemetry = Path(d, "telemetry")
            psi = Path(d, "psi-sample")
            session = Path(d, "session.json")
            proc = subprocess.Popen([
                sys.executable, str(Path(__file__).resolve().parents[1] / "codex_android_ballast.py"),
                "--ready-file", str(ready), "--status-file", str(status),
                "--telemetry-file", str(telemetry),
                "--session-stats-file", str(session),
                "--psi-status-file", str(psi),
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
                self.assertEqual(telemetry.read_text(), "16 16 0 0\n")
                psi.write_text(f"{time.time_ns() // 1000} 45000 1200\n",
                               encoding="ascii")
                proc.send_signal(signal.SIGUSR1)
                for _ in range(150):
                    # The two files are individually atomic, but not updated
                    # as a transaction. Wait for the complete new snapshot.
                    if (status.exists() and telemetry.exists()
                            and status.read_text() == "8\n"
                            and telemetry.read_text() == "8 16 8 0\n"):
                        break
                    time.sleep(0.02)
                self.assertEqual(status.read_text(), "8\n")
                self.assertEqual(telemetry.read_text(), "8 16 8 0\n")
                # Release outcome should pair the recent before/after ADB
                # PSI samples without requiring a direct /proc/pressure read.
                time.sleep(1.1)
                psi.write_text(f"{time.time_ns() // 1000} 2000 0\n",
                               encoding="ascii")
                time.sleep(1.6)
                proc.send_signal(signal.SIGUSR1)
                time.sleep(0.15)
                # The second release is intentionally more than one second
                # later and is therefore allowed to empty the reserve.
                self.assertEqual(int(status.read_text()), 0)
            finally:
                proc.terminate()
                proc.communicate(timeout=5)
            self.assertFalse(ready.exists())
            self.assertFalse(status.exists())
            self.assertFalse(telemetry.exists())
            saved = json.loads(session.read_text(encoding="utf-8"))
            self.assertTrue(saved["completed"])
            self.assertEqual(saved["initial_mb"], 16)
            self.assertGreaterEqual(saved["pressure_signals"], 1)
            self.assertIn("zero_held_seconds", saved)
            self.assertIn("release_observations", saved)
            self.assertGreaterEqual(saved["release_observations"]["paired_psi_samples"], 1)
            self.assertIn("refill_opportunity_samples", saved)
            self.assertIn("refill_blocked_available_mb_avg", saved)
            self.assertIn("empty_with_safe_headroom_samples", saved)


if __name__ == "__main__":
    unittest.main()
