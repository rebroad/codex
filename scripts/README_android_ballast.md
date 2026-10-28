# Adaptive RAM ballast for Termux Rust builds

The existing Android rustc wrapper still owns PSI, RAM/swap thresholds, suspend, and retry decisions. Ballast is a best-effort preemptive reserve, **not** an LMKD bypass or a guarantee of build completion.

## How it works

- A single controller is started by `scripts/build_codex.sh` for each guarded Cargo stage, before Cargo runs. Multiple rustc instances share it.
- It slowly preallocates and touches up to 256 MiB in 8 MiB chunks (250 ms apart). It respects a memory floor of max(12% of RAM, 256 MiB), plus 128 MiB of startup cushion.
- The post-ramp `MemAvailable` becomes the target. When available memory falls it releases chunks; when it rises it refills cautiously, with hysteresis and a 15-second release cooldown.
- The rustc wrapper sends `SIGUSR1` at half the normal averaged PSI threshold or when a critical memory/LMKD condition is observed. Signal-triggered releases are limited to 64 MiB at most once per second across all wrappers.
- The controller releases its allocations when Cargo finishes; failed initialization or zero safe headroom falls back to the existing guard.

**Limitations:** MemAvailable is an imperfect system-wide proxy, not the build's private working set. Pages can be swapped or compressed, other apps can change memory use, and PSI can still spike even if memory usage is approximately flat. Measure the effect on your device.

## Live build telemetry

While the Android memory guard is active, `build_codex.sh` prints one
consolidated ballast status line about every 15 seconds, even when Cargo
runs multiple rustc processes:

```text
[22:20:15] 🧠 Ballast: 96/256 MiB held | released 160 MiB | refilled 0 MiB | MemAvailable 824 MiB | PSI some 36.4 ms/s, full 0.0 ms/s
```

These are illustrative numbers. **Held** is the current resident
allocation *requested* by ballast (Android may swap/compress those pages),
and **released** / **refilled** are cumulative MiB unmapped / allocated
within the current Cargo stage. They can exceed the configured maximum
after repeated adjustments. `MemAvailable` is the current system-wide
kernel estimate, not Codex's RSS. PSI values are the wrapper's recent
approximately 1.25-second stall averages in milliseconds per second,
read through ADB; `n/a` means PSI is unavailable or still warming up.
These reports do not alter suspend/stop guard thresholds.

The controller keeps the original numeric `status` file for existing
wrapper behavior and updates a separate atomic `telemetry` snapshot
containing `held max released refilled` in MiB. Both files disappear
when the controller exits.

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

Keep Cargo jobs, device conditions, and profile similar. Compare elapsed time, cumulative kept/suspended/lost duration, compiler restarts, and Android kills. Fewer pauses alone do not prove an improvement.

Test the controller:

```sh
python3 -m unittest discover -s scripts/tests -p 'test_codex_android_ballast.py' -v
```


## Conservative next-build auto-tuning

On Termux, automatic ballast sizing is enabled when
`CODEX_ANDROID_RUSTC_BALLAST_MB` is **unset**. It starts at 256 MiB and
stores a history in `build/codex-android-ballast-autotune.json`.
The next Cargo stage (including the next build invocation) reads the learned
recommendation. The controller still observes the RAM floor and can allocate
less than the requested maximum when headroom is unavailable.

After a Cargo stage lasting at least 90 seconds, the tuner only changes the
reserve when that stage finished successfully **without discarded rustc work**:

- Raise by **32 MiB** (at most **384 MiB** by default) only if the reserve
  was depleted and at least two early pressure-release signals were observed.
- Lower by **32 MiB** (never below **128 MiB**) only if at most 25% of the
  reserve was needed and no pressure-release signals occurred.
- Otherwise hold the size constant. In particular, **LMKD/PSI retries do not
  cause automatic increases**, since a larger ballast could make such
  failures worse.

This is a cautious heuristic, not a learned predictor of peak Rust memory or
a guarantee against Android LMKD. The JSON file retains up to ten recent stage
observations to make decisions inspectable.

To disable learning while retaining the default size:

```sh
CODEX_ANDROID_RUSTC_BALLAST_AUTOTUNE=0 ./scripts/build_codex.sh
```

To select an exact manual size (which also disables learning for that run):

```sh
CODEX_ANDROID_RUSTC_BALLAST_MB=256 ./scripts/build_codex.sh
```

The maximum automatic reserve can be changed separately with
`CODEX_ANDROID_RUSTC_BALLAST_AUTOTUNE_CAP_MB=384`. A new build prints
whether ballast is enabled, its actual startup allocation and maximum, the
wrapper path and hash, and eventually a tuning decision when the Cargo stage
finishes. The tuned value only affects **future stages**, never the reserve
of an already-running stage.

## Rust OOM-wrapper timing

The memory guard reports cumulative, **mutually exclusive** compiler-process
durations:

```text
Rust time: kept 00:08:14; suspended 00:01:16; lost 00:10:42; in progress (this rustc) 00:01:32
```

- **Kept:** active time in completed rustc invocations whose output is retained.
- **Suspended:** time spent with rustc stopped or waiting for memory pressure.
- **Lost:** active time in invocations whose output is discarded after a failed
  or killed attempt.
- **In progress (this rustc):** active time in the current compiler attempt.
  It is not added to kept or lost until that attempt succeeds or is discarded.

Kept, suspended and lost never decrease. Timing is shared through a locked
ledger keyed to the build process, rather than each rustc reading an outdated
snapshot. With parallel compiler invocations, the totals are summed
**compiler-process seconds** and may exceed the build's wall-clock elapsed time.
The in-progress field is local to the reporting rustc, not a build-wide total.
All values refer to time, not CPU instruction counts. The split is based on
whether the wrapper keeps an invocation's output; it cannot measure partially
reusable intermediate work within a terminated rustc process.

Run the deterministic timing tests on a Linux/Termux shell with Bash and flock:

```sh
bash scripts/tests/test_codex_rustc_timing.sh
```
