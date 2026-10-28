#!/usr/bin/env python3
"""One shared, best-effort RAM ballast for a Termux Cargo build.

MemAvailable is an imperfect proxy for reclaim pressure. The ballast maintains
roughly the post-ramp MemAvailable level by returning preallocated pages as
other processes grow. It never replaces the Rust PSI/LMKD guard.
"""

import argparse
import mmap
import os
from pathlib import Path
import signal
import sys
import time

MIB = 1024 * 1024


def positive_int(value):
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return parsed


def percentage(value):
    parsed = int(value)
    if not 1 <= parsed <= 50:
        raise argparse.ArgumentTypeError("must be between 1 and 50")
    return parsed


def read_memory():
    fields = {}
    with open("/proc/meminfo", encoding="ascii") as source:
        for line in source:
            key, _, rest = line.partition(":")
            if key in ("MemTotal", "MemAvailable"):
                fields[key] = int(rest.split()[0]) * 1024
    return fields["MemTotal"], fields["MemAvailable"]


def chunk_action(available, target, held_chunks, max_chunks, floor, chunk,
                 hysteresis, can_refill):
    """Return negative chunks to release, 1 to allocate, or 0 to hold."""
    if available < floor + chunk:
        return -held_chunks
    if available < target - hysteresis:
        needed = (target - available + chunk - 1) // chunk
        return -min(held_chunks, max(1, needed), 4)
    if can_refill and held_chunks < max_chunks and available > target + hysteresis + chunk:
        return 1
    return 0


def touch_chunk(size):
    block = mmap.mmap(-1, size, access=mmap.ACCESS_WRITE)
    try:
        # Anonymous, writable mappings are not resident until touched.
        for offset in range(0, size, mmap.PAGESIZE):
            block[offset] = 1
    except BaseException:
        block.close()
        raise
    return block


def atomic_status(path, chunks, chunk_size):
    if path is None:
        return
    staging = path.with_name(path.name + f".{os.getpid()}.tmp")
    staging.write_text(str(len(chunks) * chunk_size // MIB) + "\n", encoding="ascii")
    os.replace(staging, path)


def run(args):
    ready = Path(args.ready_file)
    status = Path(args.status_file)
    chunk = args.chunk_mb * MIB
    max_chunks = args.max_mb // args.chunk_mb
    if max_chunks == 0:
        raise ValueError("max-mb must be at least chunk-mb")
    hysteresis = max(chunk, 16 * MIB)
    chunks = []
    pending_release = False
    stopping = False
    refills = 0
    releases = 0
    last_release = 0.0
    last_pressure_release = 0.0

    def on_release(_signum, _frame):
        nonlocal pending_release
        pending_release = True

    def on_stop(_signum, _frame):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGUSR1, on_release)
    signal.signal(signal.SIGTERM, on_stop)
    signal.signal(signal.SIGINT, on_stop)

    try:
        # The ballast is disposable. Android should be free to reclaim it
        # before choosing a compiler process if our voluntary release is late.
        try:
            Path("/proc/self/oom_score_adj").write_text("900\n", encoding="ascii")
        except OSError:
            pass
        total, available = read_memory()
        floor = max(total * args.floor_percent // 100, 256 * MIB)
        # Leave a separate cushion above the floor during preallocation.
        initial_chunks = min(max_chunks, max(0, (available - floor - 128 * MIB) // chunk))
        for _ in range(initial_chunks):
            if stopping:
                break
            _, available = read_memory()
            if available < floor + 128 * MIB + chunk:
                break
            chunks.append(touch_chunk(chunk))
            time.sleep(args.ramp_ms / 1000)
        _, target = read_memory()
        atomic_status(status, chunks, chunk)
        ready.write_text(str(len(chunks) * args.chunk_mb) + "\n", encoding="ascii")
        print(f"Android RAM ballast: reserved {len(chunks) * args.chunk_mb} MB "
              f"(maximum {args.max_mb} MB, minimum available {floor // MIB} MB)",
              file=sys.stderr, flush=True)
        while not stopping:
            total, available = read_memory()
            now = time.monotonic()
            if pending_release:
                pending_release = False
                # Multiple rustc wrappers can signal simultaneously. Throttle
                # release to one batch per second across the entire build.
                if now - last_pressure_release >= 1.0:
                    count = min(len(chunks), max(1, (args.pressure_release_mb + args.chunk_mb - 1) // args.chunk_mb))
                    for _ in range(count):
                        chunks.pop().close()
                    releases += count
                    last_pressure_release = now
                    last_release = now
                    atomic_status(status, chunks, chunk)
            else:
                action = chunk_action(available, target, len(chunks), max_chunks,
                                      floor, chunk, hysteresis,
                                      now - last_release >= args.refill_cooldown_s)
                if action < 0:
                    for _ in range(-action):
                        chunks.pop().close()
                    releases += -action
                    last_release = now
                    atomic_status(status, chunks, chunk)
                elif action > 0:
                    # Check the safety floor immediately before touching pages.
                    _, available = read_memory()
                    if available > floor + 128 * MIB + chunk:
                        chunks.append(touch_chunk(chunk))
                        refills += 1
                        atomic_status(status, chunks, chunk)
            time.sleep(args.interval_ms / 1000)
    finally:
        try:
            ready.unlink(missing_ok=True)
            status.unlink(missing_ok=True)
        except OSError:
            pass
        for block in chunks:
            block.close()
        print(f"Android RAM ballast: released on exit; "
              f"{releases * args.chunk_mb} MB dynamically released, "
              f"{refills * args.chunk_mb} MB refilled", file=sys.stderr, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ready-file", required=True)
    parser.add_argument("--status-file", required=True)
    parser.add_argument("--max-mb", type=positive_int, default=256)
    parser.add_argument("--chunk-mb", type=positive_int, default=8)
    parser.add_argument("--ramp-ms", type=positive_int, default=250)
    parser.add_argument("--interval-ms", type=positive_int, default=250)
    parser.add_argument("--floor-percent", type=percentage, default=12)
    parser.add_argument("--pressure-release-mb", type=positive_int, default=64)
    parser.add_argument("--refill-cooldown-s", type=positive_int, default=15)
    args = parser.parse_args()
    try:
        run(args)
    except (OSError, ValueError, KeyError, MemoryError) as exc:
        print(f"Android RAM ballast disabled: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
