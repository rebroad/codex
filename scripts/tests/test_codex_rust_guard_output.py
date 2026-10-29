#!/usr/bin/env python3
"""Tests for build-wide Rust guard output throttling and timestamps."""

import io
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from codex_rust_guard_output import CargoLogFilter, guard_status


class FakeTime:
    def __init__(self):
        self.t = 100.0

    def __call__(self):
        return self.t


class GuardOutputTest(unittest.TestCase):
    def setUp(self):
        self.clock = FakeTime()
        self.buffer = io.StringIO()
        self.f = CargoLogFilter(output=self.buffer, now=self.clock,
                                date_stamp=lambda: "2026-10-28T23:30:01+01:00")

    def test_legacy_wait_and_stop_timestamped_once_per_second(self):
        self.f.process_line(
            "stopping Rust compile before Android low-memory kill: PSI; waiting to retry")
        self.clock.t += .15
        self.f.process_line(
            "waiting for Android memory pressure to ease before (re)starting Rust compile: PSI")
        self.clock.t += .2
        self.f.process_line(
            "stopping Rust compile before Android low-memory kill: PSI; waiting to retry")
        self.assertEqual(self.buffer.getvalue().count("2026-10-28T23:30:01"), 1)
        self.assertTrue(self.buffer.getvalue().startswith(
            "[2026-10-28T23:30:01+01:00] stopping Rust compile"))
        self.clock.t += .64
        self.f.flush_due()
        self.assertEqual(len(self.buffer.getvalue().splitlines()), 1)
        self.clock.t += .02
        self.f.flush_due()
        lines = self.buffer.getvalue().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertIn("coalesced 2 rapid guard updates", lines[1])
        self.assertTrue(lines[1].startswith("[2026-10-28T23:30:01+01:00]"))

    def test_fast_suspend_resume_keeps_latest_state(self):
        self.f.process_line("⏸ Rust suspended at 23:30:01: PSI")
        self.clock.t += .2
        self.f.process_line("▶️ Rust resumed at 23:30:01 (paused 0.2s)")
        self.assertEqual(len(self.buffer.getvalue().splitlines()), 1)
        self.clock.t += .8
        self.f.flush_due()
        self.assertEqual(len(self.buffer.getvalue().splitlines()), 2)
        self.assertIn("▶️ Rust resumed", self.buffer.getvalue().splitlines()[1])

    def test_other_build_output_passes_through_immediately(self):
        self.f.process_line("⏹ Rust attempt stopped: PSI; waiting to retry")
        self.clock.t += .01
        self.f.process_line("   Compiling codex-core v0.2")
        self.assertIn("   Compiling codex-core", self.buffer.getvalue())
        self.assertEqual(self.buffer.getvalue().count("2026-10-28"), 1)
        self.assertTrue(guard_status("⏹ Rust attempt stopped at some time"))
        self.assertFalse(guard_status("   Compiling codex-core"))

    def test_lmkd_reclaim_goes_only_to_stdout_with_timestamp(self):
        stdout = io.StringIO()
        stderr = io.StringIO()
        f = CargoLogFilter(
            output=stderr, lmkd_output=stdout, now=self.clock,
            date_stamp=lambda: "2026-10-29T07:16:58+01:00")
        line = ("ℹ️ Android LMKD reclaimed com.samsung.mfcontents; "
                "advisory only (no rustc stop from this event)")
        f.process_line(line)
        self.assertIn("[2026-10-29T07:16:58+01:00] " + line,
                      stdout.getvalue())
        self.assertEqual(stderr.getvalue(), "")

    def test_pending_lmkd_advisory_cannot_be_overwritten_by_guard(self):
        stdout = io.StringIO()
        stderr = io.StringIO()
        f = CargoLogFilter(
            output=stderr, lmkd_output=stdout, now=self.clock,
            date_stamp=lambda: "2026-10-29T07:17:00+01:00")
        f.process_line("⏸ Rust suspended at now: PSI")
        self.clock.t += .2
        f.process_line(
            "ℹ️ Android LMKD reclaimed com.google.android.tts; "
            "advisory only (no rustc stop from this event)")
        self.clock.t += .1
        f.process_line("▶️ Rust resumed at now: PSI")
        self.clock.t += .7
        f.flush_due()
        self.assertEqual(stdout.getvalue().count("LMKD reclaimed"), 1)
        self.assertNotIn("LMKD reclaimed", stderr.getvalue())
        self.assertEqual(len(stderr.getvalue().splitlines()), 1)

    def test_subprocess_separates_lmkd_stdout_from_cargo_stderr(self):
        script = Path(__file__).resolve().parents[1] / "codex_rust_guard_output.py"
        result = subprocess.run(
            [sys.executable, "-u", str(script)],
            input=("   Compiling codex-app-server\n"
                   "ℹ️ Android LMKD reclaimed com.samsung.mfcontents; "
                   "advisory only (no rustc stop from this event)\n"
                   "⏸ Rust suspended at now: PSI\n"),
            capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("LMKD reclaimed com.samsung.mfcontents", result.stdout)
        self.assertNotIn("Compiling codex-app-server", result.stdout)
        self.assertNotIn("LMKD reclaimed", result.stderr)
        self.assertIn("Compiling codex-app-server", result.stderr)
        self.assertIn("Rust suspended", result.stderr)

    def test_timing_and_ballast_preserve_one_minute_rate(self):
        self.f.process_line("Rust time: kept 00:01:00; suspended 0; lost 0")
        self.f.process_line("🧠 Ballast: 32/256 MiB held")
        self.clock.t += 4
        self.f.process_line("Rust time: kept 00:01:04; suspended 0; lost 0")
        self.f.process_line("🧠 Ballast: 0/256 MiB held")
        self.clock.t += 56
        self.f.process_line("Rust time: kept 00:02:00; suspended 0; lost 0")
        self.f.process_line("🧠 Ballast: 64/256 MiB held")
        text = self.buffer.getvalue()
        self.assertEqual(text.count("Rust time: kept"), 2)
        self.assertEqual(text.count("🧠 Ballast:"), 2)
        self.assertNotIn("0/256 MiB held", text)

    def test_compact_timing_lines_preserve_one_minute_rate(self):
        self.f.process_line("14:16:30 kept=2h6m slept=3m52s lost=8m52s this=3m2s")
        self.clock.t += 5
        self.f.process_line("14:16:35 kept=2h6m5s slept=3m52s lost=8m52s this=3m7s")
        self.clock.t += 55
        self.f.process_line("14:17:30 kept=2h9m slept=3m52s lost=8m52s this=0s")
        lines = self.buffer.getvalue().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].startswith("14:16:30 kept="))
        self.assertTrue(lines[1].startswith("14:17:30 kept="))

    def test_compiler_transitions_are_never_hidden_by_timing_throttle(self):
        self.f.process_line("15:37:03 kept=24m slept=0s lost=0s this=2m")
        self.clock.t += .3
        self.f.process_line("15:37:25 ✓ kept+2m22s crate=codex_app_server pid=31337")
        self.f.process_line("15:37:25 kept=26m22s slept=0s lost=0s this=0s")
        self.f.process_line("15:37:25 ▶ start crate=core_test_support pid=31339")
        self.clock.t += .2
        self.f.process_line("15:37:41 ✓ kept+16s crate=core_test_support pid=31339")
        self.f.process_line("15:37:41 kept=26m38s slept=0s lost=0s this=0s")
        self.f.process_line("15:37:41 ▶ start crate=all pid=31340 src=app-server/tests/all.rs")
        lines = self.buffer.getvalue().splitlines()
        self.assertEqual(len(lines), 5)
        self.assertIn("kept+2m22s crate=codex_app_server pid=31337", lines[1])
        self.assertIn("start crate=core_test_support pid=31339", lines[2])
        self.assertIn("kept+16s crate=core_test_support pid=31339", lines[3])
        self.assertIn("src=app-server/tests/all.rs", lines[4])
        # There is still only one periodic aggregate snapshot in this minute.
        self.assertEqual(sum(" kept=" in line for line in lines), 1)

    def test_multiline_status_has_global_throttle(self):
        self.f.process_line("⏹ Rust attempt stopped: full PSI; waiting to retry")
        for _ in range(12):
            self.clock.t += .05
            self.f.process_line("⏸ Rust waiting at now: PSI some")
        self.assertEqual(len(self.buffer.getvalue().splitlines()), 1)
        self.clock.t += .5
        self.f.flush_due()
        self.assertEqual(len(self.buffer.getvalue().splitlines()), 2)
        self.assertIn("coalesced 12 rapid guard updates", self.buffer.getvalue())

    def test_original_stdout_fd_does_not_corrupt_captured_binary_path(self):
        # build_codex.sh captures cargo_build() stdout as an executable path.
        # Advisory notices must bypass that capture through a saved stdout fd.
        script = (
            'exec {status_fd}>&1; '
            'binary="$(printf /tmp/codex-bin; '
            'printf \'LMKD advisory\\n\' >&${status_fd})"; '
            'printf \'captured:%s\\n\' "$binary" >&2'
        )
        result = subprocess.run(
            ["bash", "-c", script], capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "LMKD advisory\n")
        self.assertIn("captured:/tmp/codex-bin", result.stderr)
        self.assertNotIn("advisory", result.stderr)

    def test_pipe_idle_flushes_queued_resume(self):
        script = Path(__file__).resolve().parents[1] / "codex_rust_guard_output.py"
        proc = subprocess.Popen([sys.executable, "-u", str(script)],
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True)
        try:
            proc.stdin.write("⏸ Rust suspended at now: PSI\n")
            proc.stdin.write("▶️ Rust resumed at now: PSI\n")
            proc.stdin.flush()
            proc.stdin.close()
            proc.wait(timeout=5)
            output = proc.stderr.read()
            self.assertEqual(proc.returncode, 0, output)
            self.assertEqual(proc.stdout.read(), "")
            self.assertIn("⏸ Rust suspended", output)
            self.assertIn("▶️ Rust resumed", output)
            self.assertEqual(len(output.splitlines()), 2)
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
