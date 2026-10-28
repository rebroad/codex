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
import selectors
import sys
import time


GUARD_PREFIXES = (
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


def guard_status(line):
    return line.startswith(GUARD_PREFIXES)


def timestamp():
    return datetime.now().astimezone().isoformat(timespec="seconds")


class CargoLogFilter:
    def __init__(self, output=None, now=None, date_stamp=None):
        self.output = output if output is not None else sys.stdout
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
        self.output.write(f"[{self.date_stamp()}] {line}{suffix}\n")
        self.output.flush()
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
                self.pending_guard = line
                self.pending_count += 1
            return

        self.flush_due(at)
        if line.startswith("Rust time: kept "):
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
    output = CargoLogFilter()
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
