# Adaptive RAM ballast for Termux Rust builds

The existing Android rustc wrapper still owns PSI, RAM/swap thresholds, suspend, and retry decisions. Ballast is a best-effort preemptive reserve, **not** an LMKD bypass or a guarantee of build completion.

## How it works

- A single controller is started by `scripts/build_codex.sh` for each guarded Cargo stage, before Cargo runs. Multiple rustc instances share it.
- It slowly preallocates and touches up to 256 MiB in 8 MiB chunks (250 ms apart). It respects a memory floor of max(12% of RAM, 256 MiB), plus 128 MiB of startup cushion.
- The post-ramp `MemAvailable` becomes the target. When available memory falls it releases chunks; when it rises it refills cautiously, with hysteresis and a 15-second release cooldown.
- The rustc wrapper sends `SIGUSR1` at half the normal averaged PSI threshold or when a critical memory/LMKD condition is observed. Signal-triggered releases are limited to 64 MiB at most once per second across all wrappers.
- The controller releases its allocations when Cargo finishes; failed initialization or zero safe headroom falls back to the existing guard.

**Limitations:** MemAvailable is an imperfect system-wide proxy, not the build's private working set. Pages can be swapped or compressed, other apps can change memory use, and PSI can still spike even if memory usage is approximately flat. Measure the effect on your device.

## Settings

| Environment variable | Default | Description |
| --- | --- | --- |
| `CODEX_ANDROID_RUSTC_BALLAST_MB` | `256` | Max ballast MiB; **0 disables** |
| `CODEX_ANDROID_RUSTC_BALLAST_CHUNK_MB` | `8` | Chunk size in MiB |
| `CODEX_ANDROID_RUSTC_BALLAST_RAMP_MS` | `250` | Time between initial allocations |
| `CODEX_ANDROID_RUSTC_BALLAST_FLOOR_PERCENT` | `12` | Minimum percentage of RAM to leave available |

Example:

```sh
CODEX_ANDROID_RUSTC_BALLAST_MB=128 ./scripts/build_codex.sh
```

Baseline comparison:

```sh
CODEX_ANDROID_RUSTC_BALLAST_MB=0 ./scripts/build_codex.sh
```

Keep Cargo jobs, device conditions, and profile similar. Compare elapsed time, cumulative running/paused duration, compiler restarts, and Android kills. Fewer pauses alone do not prove an improvement.

Test the controller:

```sh
python3 -m unittest discover -s scripts/tests -p 'test_codex_android_ballast.py' -v
```
