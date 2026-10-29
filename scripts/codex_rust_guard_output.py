#!/usr/bin/env python3
"""Stream Cargo logs with build-wide Rust guard throttling and timestamps.

Every rustc wrapper feeds this single pipeline. Status changes are printed at
most once per second in aggregate, using a monotonic clock; intervening changes
are coalesced into the most recent status, not repeated on every PSI poll.
Cargo diagnostics are forwarded without throttling.
"""

import codecs
from datetime import datetime
import os
import re
import selectors
import sys
import time


LMKD_ADVISORY_PREFIX = "ℹ️ Android LMKD reclaimed "

GUARD_PREFIXES = (
    LMKD_ADVISORY_PREFIX,
    "stopping Rust compile before Android low-memory kill:",
    "waiting for Android memory pressure to ease",
    "⏹ Rust attempt stopped",
    "⏸ Rust waiting",
    "⏸ Rust suspended",
    "⏸ Rust paused",
    "▶️ Rust resumed",
    "▶️ Rust running",
    "▶️ Rust started",
    "🔄 Rust state changed",
    "🔁 Rust state flapped",
    "Android LMKD reclaim quiet for",
    "suspending Rust compile",
    "resuming Rust compile",
    "Android Rust compile state changed",
    "Android Rust compile state flapped",
)

GUARD_MIN_INTERVAL = 1.0
PROGRESS_MIN_INTERVAL = 60.0
COMPACT_TIMING = re.compile(
    r"^\d{2}:\d{2}:\d{2} kept=\d+(?:h|m|s)")


def guard_status(line):
    return line.startswith(GUARD_PREFIXES)


def timestamp():
    return datetime.now().astimezone().isoformat(timespec="seconds")


class CargoLogFilter:
    def __init__(self, output=None, now=None, date_stamp=None, lmkd_output=None):
        self.output = output if output is not None else sys.stdout
        self.lmkd_output = lmkd_output if lmkd_output is not None else self.output
        self.now = now if now is not None else time.monotonic
        self.date_stamp = date_stamp if date_stamp is not None else timestamp
        self.last_guard_at = None
        self.last_timing_at = None
        self.last_ballast_at = None
        self.pending_guard = None
        self.pending_count = 0

    def emit_guard(self, line, at, coalesced=0):
        suffix = (f" (coalesced {coalesced} rapid guard updates)"
                  if coalesced else "")
        destination = (self.lmkd_output if line.startswith(LMKD_ADVISORY_PREFIX)
                       else self.output)
        destination.write(f"[{self.date_stamp()}] {line}{suffix}\n")
        destination.flush()
        self.last_guard_at = at

    def flush_due(self, at=None):
        if at is None:
            at = self.now()
        if (self.pending_guard is not None
                and (self.last_guard_at is None
                     or at - self.last_guard_at >= GUARD_MIN_INTERVAL)):
            self.emit_guard(self.pending_guard, at, self.pending_count)
            self.pending_guard = None
            self.pending_count = 0

    def process_line(self, line):
        at = self.now()
        if guard_status(line):
            if (self.last_guard_at is None
                    or at - self.last_guard_at >= GUARD_MIN_INTERVAL):
                # A newer state supersedes any previous, not-yet-reported state.
                self.emit_guard(line, at, self.pending_count)
                self.pending_guard = None
                self.pending_count = 0
            else:
                # Do not let rapid suspend/resume messages replace an LMKD
                # notice that is still waiting for its one-second log slot.
                # LMKD is diagnostic, so its destination is the build stdout.
                if (line.startswith(LMKD_ADVISORY_PREFIX)
                        or self.pending_guard is None
                        or not self.pending_guard.startswith(LMKD_ADVISORY_PREFIX)):
                    self.pending_guard = line
                self.pending_count += 1
            return

        self.flush_due(at)
        if line.startswith("Rust time: kept ") or COMPACT_TIMING.match(line):
            if (self.last_timing_at is not None
                    and at - self.last_timing_at < PROGRESS_MIN_INTERVAL):
                return
            self.last_timing_at = at
        elif line.startswith("🧠 Ballast:"):
            if (self.last_ballast_at is not None
                    and at - self.last_ballast_at < PROGRESS_MIN_INTERVAL):
                return
            self.last_ballast_at = at
            line = f"[{datetime.now().astimezone():%H:%M:%S}] {line}"
        self.output.write(line + "\n")
        self.output.flush()

    def finish(self):
        # Do not discard the latest transition (e.g. a quick resume right
        # before Cargo exits) or violate the one-second constraint at EOF.
        if self.pending_guard is not None:
            delay = GUARD_MIN_INTERVAL - (self.now() - self.last_guard_at)
            if delay > 0:
                time.sleep(delay)
            self.flush_due()


def run():
    # Cargo/rustc stdout is not touched: the build combines Cargo output
    # into our stdin first. Only informational LMKD reclaims go to the
    # build script's stdout; compiler logs and ordinary guard events remain
    # on stderr as before.
    output = CargoLogFilter(output=sys.stderr, lmkd_output=sys.stdout)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    pending_text = ""
    stdin_fd = sys.stdin.fileno()
    with selectors.DefaultSelector() as selector:
        selector.register(stdin_fd, selectors.EVENT_READ)
        while True:
            timeout = None
            if output.pending_guard is not None:
                timeout = max(
                    0.0,
                    GUARD_MIN_INTERVAL - (time.monotonic() - output.last_guard_at))
            events = selector.select(timeout)
            if not events:
                output.flush_due()
                continue
            data = os.read(stdin_fd, 65536)
            if not data:
                pending_text += decoder.decode(b"", final=True)
                if pending_text:
                    output.process_line(pending_text)
                output.finish()
                return
            pending_text += decoder.decode(data)
            while "\n" in pending_text:
                line, pending_text = pending_text.split("\n", 1)
                output.process_line(line.rstrip("\r"))


if __name__ == "__main__":
    run()
