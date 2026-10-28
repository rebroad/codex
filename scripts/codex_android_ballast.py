#!/usr/bin/env python3
"""One shared, best-effort RAM ballast for a Termux Cargo build.

MemAvailable is an imperfect proxy for reclaim pressure. The ballast maintains
roughly the post-ramp MemAvailable level by returning preallocated pages as
other processes grow. It never replaces the Rust PSI/LMKD guard.
"""

import argparse
import fcntl
import json
import mmap
import os
from pathlib import Path
import signal
import sys
import time

MIB = 1024 * 1024
TUNE_STEP_MB = 32
TUNE_MIN_MB = 128


def tune_next_mb(current_mb, initial_mb, lowest_mb, pressure_signals,
                 elapsed_s, cargo_success, lost_us, cap_mb):
    """Conservative one-step adaptation; never increase after discarded work.

    The ballast cannot produce RAM. An exhausted ballast and severe PSI may
    indicate sustained pressure, so failed/restarted builds must not teach
    the tuner to allocate even more.
    """
    current_mb = max(TUNE_MIN_MB, min(current_mb, cap_mb))
    if not cargo_success or lost_us > 0 or elapsed_s < 90:
        return current_mb, "unchanged (insufficient clean evidence)"
    if initial_mb < current_mb - TUNE_STEP_MB:
        return current_mb, "unchanged (RAM floor limited preallocation)"
    used_mb = max(0, initial_mb - lowest_mb)
    if pressure_signals >= 2 and lowest_mb <= TUNE_STEP_MB and current_mb < cap_mb:
        return min(cap_mb, current_mb + TUNE_STEP_MB), "increased (clean build exhausted reserve)"
    if pressure_signals == 0 and used_mb <= current_mb // 4 and current_mb > TUNE_MIN_MB:
        return max(TUNE_MIN_MB, current_mb - TUNE_STEP_MB), "decreased (reserve mostly unused)"
    return current_mb, "unchanged (reserve matched observed demand)"


def read_tuning(path, base_mb, cap_mb, system_ram_mb):
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        selected = payload.get("recommended_mb")
        observed_ram = payload.get("system_ram_mb")
        if (isinstance(selected, int) and not isinstance(selected, bool)
                and isinstance(observed_ram, int)
                and abs(observed_ram - system_ram_mb) <= system_ram_mb // 10):
            return max(TUNE_MIN_MB, min(selected, cap_mb))
    except (OSError, ValueError, TypeError, AttributeError):
        pass
    return base_mb


def record_tuning(path, result_file, stats, cap_mb):
    if path is None or result_file is None or not result_file.exists():
        return
    try:
        cargo_status, lost_us = map(int, result_file.read_text(encoding="ascii").split())
        if cargo_status < 0 or lost_us < 0:
            return
    except (OSError, ValueError):
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    # Separate, persistent lock handles accidental overlapping Cargo stages.
    with open(str(path) + ".lock", "a+b") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        previous = {}
        try:
            previous = json.loads(path.read_text(encoding="utf-8"))
            if not isinstance(previous, dict):
                previous = {}
        except (OSError, ValueError):
            pass
        proposed, reason = tune_next_mb(
            stats["max_mb"], stats["initial_mb"], stats["lowest_mb"],
            stats["pressure_signals"], stats["elapsed_s"],
            cargo_status == 0, lost_us, cap_mb)
        # Avoid promoting stale device-specific observations.
        if previous.get("system_ram_mb") != stats["system_ram_mb"]:
            previous = {}
        observations = previous.get("observations", [])
        if not isinstance(observations, list):
            observations = []
        observations = (observations + [dict(stats, cargo_status=cargo_status,
                                               lost_us=lost_us, decision=reason)])[-10:]
        result = dict(version=1, system_ram_mb=stats["system_ram_mb"],
                      recommended_mb=proposed, observations=observations)
        temp = path.with_name(path.name + f".{os.getpid()}.tmp")
        temp.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        os.replace(temp, path)
        print(f"Android RAM ballast tuning: next reserve {proposed} MiB "
              f"({reason}; {stats['initial_mb']} MiB allocated initially, "
              f"{stats['initial_mb']-stats['lowest_mb']} MiB peak released)",
              file=sys.stderr, flush=True)


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


def atomic_status(path, chunks, chunk_size, telemetry_path=None, max_mb=0,
                  released_chunks=0, refilled_chunks=0):
    # Keep the single-number status file for existing rustc wrappers. Write
    # separate, atomically replaced telemetry for richer build output.
    held_mb = len(chunks) * chunk_size // MIB
    staging = path.with_name(path.name + f".{os.getpid()}.tmp")
    staging.write_text(str(held_mb) + "\n", encoding="ascii")
    os.replace(staging, path)
    if telemetry_path is not None:
        telemetry_staging = telemetry_path.with_name(
            telemetry_path.name + f".{os.getpid()}.tmp")
        telemetry_staging.write_text(
            f"{held_mb} {max_mb} "
            f"{released_chunks * chunk_size // MIB} "
            f"{refilled_chunks * chunk_size // MIB}\n",
            encoding="ascii")
        os.replace(telemetry_staging, telemetry_path)



def read_psi_sample(path, now_wall=None):
    """Return recent (timestamp_us, some_us/s, full_us/s) from Rust's ADB PSI.

    No direct /proc/pressure read: Android/Termux may not allow it.
    """
    if path is None:
        return None
    try:
        timestamp, some, full = map(int, path.read_text(encoding="ascii").split())
        now_us = int((time.time() if now_wall is None else now_wall) * 1000000)
        if (0 <= now_us - timestamp <= 4_000_000
                and 0 <= some <= 1_000_000 and 0 <= full <= 1_000_000):
            return timestamp, some, full
    except (OSError, ValueError):
        pass
    return None


def recovery_refill_decision(available, floor, chunk, held_chunks, max_chunks,
                             seconds_since_release, seconds_since_pressure,
                             psi_sample, cooldown_s=15):
    """Can we cautiously recover an emptied reserve, ignoring an old target?

    Release decisions remain unchanged. A refill never violates the available
    RAM floor, always observes a quiet period, and respects sampled PSI when
    ADB is available. Without PSI, wait longer and require more headroom.
    """
    if held_chunks >= max_chunks:
        return False, "full"
    if seconds_since_release < cooldown_s:
        return False, "cooldown"
    if psi_sample is None:
        if seconds_since_pressure < 60:
            return False, "quiet"
        if available < floor + 1024 * MIB + chunk:
            return False, "headroom"
    else:
        if psi_sample[1] >= 10_000 or psi_sample[2] >= 2_000:
            return False, "psi"
        if seconds_since_pressure < 30:
            return False, "quiet"
        if available < floor + 512 * MIB + chunk:
            return False, "headroom"
    return True, "safe"


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + f".{os.getpid()}.tmp")
    tmp.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n",
                   encoding="utf-8")
    os.replace(tmp, path)


def run(args):
    ready = Path(args.ready_file)
    status = Path(args.status_file)
    telemetry = Path(args.telemetry_file) if args.telemetry_file else None
    psi_path = Path(args.psi_status_file) if args.psi_status_file else None
    session_path = Path(args.session_stats_file) if args.session_stats_file else None
    history = Path(args.history_file) if args.history_file else None
    result_file = Path(args.build_result_file) if args.build_result_file else None
    system_ram, _ = read_memory()
    if history is not None:
        args.max_mb = read_tuning(history, args.max_mb, args.autotune_cap_mb,
                                  system_ram // MIB)
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
    pressure_signals = 0
    last_pressure_signal = time.monotonic()
    last_refill = 0.0
    initial_mb = 0
    lowest_mb = 0
    controller_start = time.monotonic()
    target = 0
    floor = max(system_ram * args.floor_percent // 100, 256 * MIB)

    # These counters are updated without log messages; session snapshots are
    # written every 30s so diagnostics survive a later process kill.
    zero_seconds = 0.0
    longest_zero_seconds = 0.0
    zero_periods = 0
    zero_since = None
    last_measure = controller_start
    last_held_count = 0
    last_session_snapshot = controller_start
    last_refill_observation = controller_start
    refill_opportunity_samples = 0
    refill_opportunity_available_mb_sum = 0
    refill_opportunity_min_available_mb = None
    refill_opportunity_max_available_mb = None
    blocked_refill_samples = 0
    blocked_refill_available_mb_sum = 0
    blocked_refill_available_mb_min = None
    blocked_refill_available_mb_max = None
    empty_with_safe_headroom_samples = 0
    refill_blocked_by_target_samples = 0
    refill_blocked_by_quiet_samples = 0
    refill_blocked_by_psi_samples = 0
    refill_blocked_by_cooldown_samples = 0
    refill_safety_headroom_samples = 0
    refill_recovery_events = 0

    release_events = []
    pending_after_release = []
    paired_psi_releases = 0
    unpaired_psi_releases = 0
    before_some_sum = 0
    after_some_sum = 0
    before_full_sum = 0
    after_full_sum = 0
    last_release_reason = "none"

    def publish_status():
        atomic_status(status, chunks, chunk, telemetry, args.max_mb,
                      releases, refills)

    def update_empty_time(now):
        nonlocal last_measure, last_held_count, zero_since, zero_seconds
        nonlocal longest_zero_seconds, zero_periods
        if last_held_count == 0:
            zero_seconds += max(0.0, now - last_measure)
        last_measure = now
        held = len(chunks)
        if held == 0 and last_held_count > 0:
            zero_since = now
            zero_periods += 1
        elif held > 0 and last_held_count == 0 and zero_since is not None:
            longest_zero_seconds = max(longest_zero_seconds, now - zero_since)
            zero_since = None
        last_held_count = held

    def released(source, count, available, now):
        nonlocal releases, last_release, lowest_mb, last_release_reason
        if count <= 0:
            return
        before = read_psi_sample(psi_path)
        event = {
            "elapsed_s": round(now - controller_start, 1),
            "cause": source,
            "released_mb": count * args.chunk_mb,
            "available_before_mb": available // MIB,
            "psi_some_before_ms_s": round(before[1] / 1000, 2) if before else None,
            "psi_full_before_ms_s": round(before[2] / 1000, 2) if before else None,
            "psi_some_after_ms_s": None,
            "psi_full_after_ms_s": None,
            "available_after_mb": None,
        }
        release_events.append(event)
        del release_events[:-40]
        pending_after_release.append((now, time.time_ns() // 1000, before, event))
        releases += count
        last_release = now
        last_release_reason = source
        lowest_mb = min(lowest_mb, len(chunks) * args.chunk_mb)
        publish_status()

    def complete_release_samples(now, available, psi):
        nonlocal paired_psi_releases, unpaired_psi_releases
        nonlocal before_some_sum, after_some_sum, before_full_sum, after_full_sum
        still_pending = []
        for start, event_epoch_us, before, event in pending_after_release:
            if (now - start >= 2.0 and before and psi is not None
                    and psi[0] >= event_epoch_us + 500_000):
                event["psi_some_after_ms_s"] = round(psi[1] / 1000, 2)
                event["psi_full_after_ms_s"] = round(psi[2] / 1000, 2)
                event["available_after_mb"] = available // MIB
                paired_psi_releases += 1
                before_some_sum += before[1]
                before_full_sum += before[2]
                after_some_sum += psi[1]
                after_full_sum += psi[2]
            elif now - start >= 6.0:
                event["available_after_mb"] = available // MIB
                unpaired_psi_releases += 1
            else:
                still_pending.append((start, event_epoch_us, before, event))
        pending_after_release[:] = still_pending

    def snapshot(now, final=False):
        duration = max(0.0, now - controller_start)
        longest = max(longest_zero_seconds,
                      now - zero_since if zero_since is not None else 0.0)
        observations = {
            "version": 2,
            "completed": final,
            "system_ram_mb": system_ram // MIB,
            "max_mb": args.max_mb,
            "initial_mb": initial_mb,
            "current_held_mb": len(chunks) * args.chunk_mb,
            "lowest_mb": lowest_mb,
            "target_available_mb": target // MIB,
            "memory_floor_mb": floor // MIB,
            "pressure_signals": pressure_signals,
            "released_mb": releases * args.chunk_mb,
            "refilled_mb": refills * args.chunk_mb,
            "elapsed_s": int(duration),
            "zero_held_seconds": round(zero_seconds, 1),
            "zero_held_fraction": round(zero_seconds / duration, 4) if duration else 0,
            "longest_zero_held_seconds": round(longest, 1),
            "zero_held_periods": zero_periods,
            "refill_opportunity_samples": refill_opportunity_samples,
            "refill_opportunity_available_mb_avg": (
                round(refill_opportunity_available_mb_sum / refill_opportunity_samples)
                if refill_opportunity_samples else None),
            "refill_opportunity_available_mb_min": refill_opportunity_min_available_mb,
            "refill_opportunity_available_mb_max": refill_opportunity_max_available_mb,
            "refill_blocked_samples": blocked_refill_samples,
            "refill_blocked_available_mb_avg": (
                round(blocked_refill_available_mb_sum / blocked_refill_samples)
                if blocked_refill_samples else None),
            "refill_blocked_available_mb_min": blocked_refill_available_mb_min,
            "refill_blocked_available_mb_max": blocked_refill_available_mb_max,
            "empty_with_safe_headroom_samples": empty_with_safe_headroom_samples,
            "refill_blocked_by_original_target_samples": refill_blocked_by_target_samples,
            "refill_blocked_by_quiet_samples": refill_blocked_by_quiet_samples,
            "refill_blocked_by_psi_samples": refill_blocked_by_psi_samples,
            "refill_blocked_by_cooldown_samples": refill_blocked_by_cooldown_samples,
            "refill_safety_headroom_samples": refill_safety_headroom_samples,
            "adaptive_refill_events": refill_recovery_events,
            "last_release_reason": last_release_reason,
            "release_observations": {
                "paired_psi_samples": paired_psi_releases,
                "unpaired_psi_samples": unpaired_psi_releases,
                # Averages are observed PSI rate before/2+ seconds after
                # release; they are correlations, not causal measurements.
                "some_before_avg_ms_s": round(before_some_sum / paired_psi_releases / 1000, 2)
                    if paired_psi_releases else None,
                "some_after_avg_ms_s": round(after_some_sum / paired_psi_releases / 1000, 2)
                    if paired_psi_releases else None,
                "full_before_avg_ms_s": round(before_full_sum / paired_psi_releases / 1000, 2)
                    if paired_psi_releases else None,
                "full_after_avg_ms_s": round(after_full_sum / paired_psi_releases / 1000, 2)
                    if paired_psi_releases else None,
                "recent_events": [dict(e) for e in release_events],
            },
        }
        return observations

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
        # Ballast is deliberately a higher-priority LMKD victim than rustc.
        try:
            Path("/proc/self/oom_score_adj").write_text("900\n", encoding="ascii")
        except OSError:
            pass
        total, available = read_memory()
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
        initial_mb = len(chunks) * args.chunk_mb
        lowest_mb = initial_mb
        last_measure = time.monotonic()
        last_held_count = len(chunks)
        if not chunks:
            zero_since = last_measure
            zero_periods = 1
        publish_status()
        ready.write_text(str(initial_mb) + "\n", encoding="ascii")
        print(f"Android RAM ballast: reserved {initial_mb} MB "
              f"(maximum {args.max_mb} MB, minimum available {floor // MIB} MB; "
              f"adaptive refill enabled)", file=sys.stderr, flush=True)

        while not stopping:
            _, available = read_memory()
            now = time.monotonic()
            psi = read_psi_sample(psi_path)
            complete_release_samples(now, available, psi)

            if pending_release:
                pending_release = False
                pressure_signals += 1
                last_pressure_signal = now
                # Respond immediately to memory pressure, but limit aggregate
                # release requests to one batch per second across all rustc.
                if now - last_pressure_release >= 1.0:
                    count = min(len(chunks), max(1, (args.pressure_release_mb
                                + args.chunk_mb - 1) // args.chunk_mb))
                    for _ in range(count):
                        chunks.pop().close()
                    if count:
                        last_pressure_release = now
                        released("psi_signal", count, available, now)
            else:
                safe, gate = recovery_refill_decision(
                    available, floor, chunk, len(chunks), max_chunks,
                    now - last_release, now - last_pressure_signal,
                    psi, args.refill_cooldown_s)

                # Once a second, measure whether there was adequate safe RAM
                # to refill, including situations blocked by the original
                # post-ramp target. Counts are sample counts, not seconds.
                if now - last_refill_observation >= 1.0:
                    last_refill_observation = now
                    if len(chunks) < max_chunks:
                        if available > floor + 512 * MIB + chunk:
                            mb = available // MIB
                            refill_opportunity_samples += 1
                            refill_opportunity_available_mb_sum += mb
                            refill_opportunity_min_available_mb = (
                                mb if refill_opportunity_min_available_mb is None
                                else min(mb, refill_opportunity_min_available_mb))
                            refill_opportunity_max_available_mb = (
                                mb if refill_opportunity_max_available_mb is None
                                else max(mb, refill_opportunity_max_available_mb))
                            target_blocks = available <= target + hysteresis + chunk
                            if target_blocks:
                                refill_blocked_by_target_samples += 1
                            if not safe or target_blocks:
                                # Actual RAM observed when a refill could have
                                # fitted but the old target or pressure policy
                                # prevented it. Keep units and sample counts.
                                blocked_refill_samples += 1
                                blocked_refill_available_mb_sum += mb
                                blocked_refill_available_mb_min = (
                                    mb if blocked_refill_available_mb_min is None
                                    else min(mb, blocked_refill_available_mb_min))
                                blocked_refill_available_mb_max = (
                                    mb if blocked_refill_available_mb_max is None
                                    else max(mb, blocked_refill_available_mb_max))
                            if not chunks:
                                empty_with_safe_headroom_samples += 1
                            if gate == "quiet":
                                refill_blocked_by_quiet_samples += 1
                            elif gate == "psi":
                                refill_blocked_by_psi_samples += 1
                            elif gate == "cooldown":
                                refill_blocked_by_cooldown_samples += 1
                        else:
                            refill_safety_headroom_samples += 1

                # The original post-allocation MemAvailable target can remain
                # permanently above RAM available to subsequent rustc stages.
                # Once PSI is quiet and the safety cushion is large, rebase it
                # downward so refills can occur without immediately releasing
                # the same chunks on the next controller poll.
                if safe:
                    newer_target = max(floor + 128 * MIB,
                                       available - max(64 * MIB, 4 * chunk))
                    if newer_target < target:
                        target = newer_target

                action = chunk_action(
                    available, target, len(chunks), max_chunks, floor, chunk,
                    hysteresis, safe and now - last_refill >= 1.0)
                if action < 0:
                    count = min(len(chunks), -action)
                    for _ in range(count):
                        chunks.pop().close()
                    released("memavailable_target", count, available, now)
                elif action > 0:
                    # Recheck headroom and fresh PSI immediately before any
                    # memory-touching allocation, never more than 8 MiB/s.
                    _, available = read_memory()
                    latest_psi = read_psi_sample(psi_path)
                    still_safe, _ = recovery_refill_decision(
                        available, floor, chunk, len(chunks), max_chunks,
                        now - last_release, now - last_pressure_signal,
                        latest_psi, args.refill_cooldown_s)
                    if still_safe and available > floor + 128 * MIB + chunk:
                        chunks.append(touch_chunk(chunk))
                        refills += 1
                        last_refill = now
                        refill_recovery_events += 1
                        publish_status()

            update_empty_time(now)
            if session_path is not None and now - last_session_snapshot >= 30:
                try:
                    atomic_json(session_path, snapshot(now))
                except OSError as exc:
                    # Diagnostics must not make an otherwise healthy ballast fail.
                    print(f"Android RAM ballast stats checkpoint unavailable: {exc}",
                          file=sys.stderr, flush=True)
                last_session_snapshot = now
            time.sleep(args.interval_ms / 1000)
    finally:
        now = time.monotonic()
        update_empty_time(now)
        try:
            if session_path is not None:
                atomic_json(session_path, snapshot(now, final=True))
        except OSError as exc:
            print(f"Android RAM ballast stats unavailable: {exc}",
                  file=sys.stderr, flush=True)
        try:
            ready.unlink(missing_ok=True)
            status.unlink(missing_ok=True)
            if telemetry is not None:
                telemetry.unlink(missing_ok=True)
        except OSError:
            pass
        for block in chunks:
            block.close()
        print(f"Android RAM ballast: released on exit; "
              f"{releases * args.chunk_mb} MB dynamically released, "
              f"{refills * args.chunk_mb} MB refilled; "
              f"{zero_seconds:.0f}s empty, "
              f"{paired_psi_releases} paired PSI releases",
              file=sys.stderr, flush=True)
        try:
            record_tuning(history, result_file, snapshot(now, final=True),
                          args.autotune_cap_mb)
        except (OSError, ValueError) as exc:
            print(f"Android RAM ballast tuning unavailable: {exc}",
                  file=sys.stderr, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ready-file", required=True)
    parser.add_argument("--status-file", required=True)
    parser.add_argument("--telemetry-file", help="atomic held/max/released/refilled MiB snapshot")
    parser.add_argument("--psi-status-file", help="recent ADB PSI sampled by Rust guard")
    parser.add_argument("--session-stats-file", help="periodic persistent ballast measurements")
    parser.add_argument("--history-file", help="persistent next-build autotuning history")
    parser.add_argument("--build-result-file", help="status and lost microseconds recorded after Cargo exits")
    parser.add_argument("--autotune-cap-mb", type=positive_int, default=384)
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
