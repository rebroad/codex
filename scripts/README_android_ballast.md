# Adaptive RAM ballast for Termux Rust builds

The existing Android rustc wrapper still owns PSI, RAM/swap thresholds, suspend, and retry decisions. Ballast is a best-effort preemptive reserve, **not** an LMKD bypass or a guarantee of build completion.

## How it works

- A single controller is started by `scripts/build_codex.sh` for each guarded Cargo stage, before Cargo runs. Multiple rustc instances share it.
- It slowly preallocates and touches up to 256 MiB in 8 MiB chunks (250 ms apart). It respects a memory floor of max(12% of RAM, 256 MiB), plus 128 MiB of startup cushion.
- The post-ramp `MemAvailable` becomes the initial target. Automatic release follows falling available RAM; after a pressure lull, the controller may **lower an obsolete target** and cautiously replenish its reserve, instead of remaining at 0 MiB indefinitely.
- Recovery refill waits at least 15 seconds after a release **and** 30 seconds after a pressure signal with recent low PSI (below 10 ms/s some and 2 ms/s full). It requires 512 MiB of additional RAM cushion above the configured floor. If recent ADB PSI is unavailable, it instead waits 60 seconds and requires 1024 MiB of extra cushion. Refill is limited to one 8 MiB chunk per second by default.
- The rustc wrapper sends `SIGUSR1` at half the normal averaged PSI threshold or when a critical memory/LMKD condition is observed. Signal-triggered releases are limited to 64 MiB at most once per second across all wrappers.
- The controller releases its allocations when Cargo finishes; failed initialization or zero safe headroom falls back to the existing guard.

**Limitations:** MemAvailable is an imperfect system-wide proxy, not the build's private working set. Pages can be swapped or compressed, other apps can change memory use, and PSI can still spike even if memory usage is approximately flat. Measure the effect on your device.

## Live build telemetry

While the Android memory guard is active, `build_codex.sh` prints
a ballast snapshot together with a **Kept / Suspended / Lost** Rust timing
report. The build-wide output filter limits both to **at most one update
per minute**, even when Cargo runs multiple rustc processes. Pause/resume
events themselves are emitted once at each state transition, not periodically.
The controller can continue adjusting memory every 250 ms without flooding
the terminal. **All Rust guard status messages** (including waiting-to-retry,
waiting-for-memory, suspended and resumed messages) are prefixed with a local
timestamp and limited to **one visible message per second across the entire
Cargo stage**. Rapid state updates are combined into the most recent status
with a count of coalesced updates. Cargo compiler diagnostics are not
throttled:

```text
14:16:30 kept=8m14s slept=1m16s lost=10m42s this=1m32s
[22:20:15] 🧠 Ballast: 96/256 MiB held | released 160 MiB | refilled 0 MiB | MemAvailable 824 MiB | PSI some 36.4 ms/s, full 0.0 ms/s
```

These are illustrative numbers. Output is written to stderr, which the build script
captures alongside stdout in its build log. **Held** is the current resident
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

## Persistent diagnostics and next-build learning

Auto-tuning is **enabled by default** when
`CODEX_ANDROID_RUSTC_BALLAST_MB` has not been explicitly set. The controller
also writes
`build/codex-android-ballast-last-session.json` every **30 seconds**
and once at normal shutdown, whether the Cargo stage succeeds or fails.
This checkpoint may survive a later abrupt LMKD kill. The file is overwritten
by the next Cargo stage, while completed stages are retained (last ten) in
`build/codex-android-ballast-autotune.json`.

The detailed JSON records:

- **Zero-reserve time:** total seconds, share of elapsed stage time, count and
  longest interval with no pages held.
- **Release outcomes:** pressure-signal versus MemAvailable-target releases;
  available RAM and ADB PSI *before* and approximately 2 seconds *after*
  release, when both fresh observations exist. Missing/invalid PSI is recorded
  as missing rather than 0; only paired observations contribute to before/after
  averages. This is observational, **not** proof that the release caused a PSI
  change. The last 40 release examples are retained alongside counts.
- **Refill opportunities:** once-per-second samples of RAM available while the
  reserve was below capacity, including samples blocked by the original
  target, cooldown, recent pressure, high PSI, or insufficient RAM headroom.
  Available RAM minima, maxima and averages are retained.

The next-build **size** tuner remains deliberately conservative: it does not
increase allocation after discarded compiler work, because increasing a
reserve cannot create more physical memory. The **within-build** refill
recovery works independently of that size decision. This means even builds
that experience OOM retries contribute useful diagnostics, without teaching
the size tuner to chase repeated crashes.

The controller reads PSI snapshots from the existing Rust guard's ADB stream,
using a shared atomic file. It does **not** require direct access to Android's
restricted `/proc/pressure/memory`. The controller never prints a line for
every chunk release or refill; the normal ballast and Rust timing reports stay
at approximately one per minute.

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
a guarantee against Android LMKD. The history file retains up to ten stage
observations including empty-reserve time, refill-blocking reasons, and paired
pressure samples to support improving the policy based on actual results.

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

## Rust memory-pressure protection

PSI measures **stall time**, not how much RAM or swap remains. The earlier
policy terminated a Rust compilation whenever system-wide memory PSI `full`
exceeded 60 ms/s, even if available RAM was plentiful. That could repeatedly
discard minutes of progress without preventing an Android LMKD kill.

The guard now distinguishes **reversible pressure** from **hard memory danger**:

- PSI `some` >= 35 ms/s and PSI `full` >= 60 ms/s both trigger a **reversible
  SIGSTOP** when outside the recovery-grace window. Neither PSI value on its
  own is a reason to terminate the compiler.
- The compiler can resume when PSI subsides, as before. Even if PSI stays
  high, a PSI-only pause lasts **at most 5 seconds** by default and is then
  resumed, with **30 seconds** of PSI-only suspension grace so Rust can make
  forward progress. A compiler waiting to start is likewise admitted after
  a five-second PSI-only wait.
- Critically low `MemAvailable` and insufficient combined RAM/swap headroom
  **still allow immediate stop and retry**, even during PSI grace. An actual
  unexpected rustc exit is also reported to Cargo.
- **LMKD reclaim of another Android app is advisory, never an automatic rustc
  hard stop or a reason to wait for a five-check quiet period.** The guard
  requests immediate relief from any remaining ballast, keeps the compiler's
  work, and continues evaluating real RAM/swap danger and reversible PSI.
- Each LMKD notice is written by the rustc wrapper to **stderr** (to keep
  Cargo's compiler output protocol clean), then the build-wide log filter
  timestamps and forwards advisories to the **build's original stdout**.
  This deliberately bypasses the command substitution that captures
  `cargo_build()`'s binary path. All other Cargo output stays on stderr.
  Rapid notices share the existing one-per-second status rate limit.
- The ballast still gets requests to release reserved memory before the
  ordinary PSI thresholds, independently of these compiler actions.

The two backoff periods are configurable with
`CODEX_ANDROID_RUSTC_PSI_MAX_PAUSE_SECONDS=5` and
`CODEX_ANDROID_RUSTC_PSI_RESUME_GRACE_SECONDS=30`. Tuning them does **not**
alter the hard memory headroom thresholds. This favors retaining already
completed compiler work under reclaim stalls but cannot prevent Android
itself from killing a process under real memory exhaustion.

Run the pause-policy and LMKD advisory regression tests:

```sh
bash scripts/tests/test_codex_rustc_suspend_state.sh
bash scripts/tests/test_codex_rustc_lmkd_advisory.sh
```

## Rust OOM-wrapper timing

The memory guard reports cumulative, **mutually exclusive** compiler-process
durations:

```text
14:16:30 kept=8m14s slept=1m16s lost=10m42s this=1m32s
```

- **Kept:** active time in completed rustc invocations whose output is retained.
- **Suspended:** time spent with rustc stopped or waiting for memory pressure.
- **Lost:** active time in invocations whose output is discarded after a failed
  or killed attempt.
- **In progress (this rustc):** active time in the current compiler attempt.
  It is not added to kept or lost until that attempt succeeds or is discarded.

**Kept, slept, and lost reset to zero at the start of each top-level
`just test` or `build_codex.sh` invocation.** All compiler wrappers
belonging to that command update the same `flock`-protected run ledger.
The shared setup in `scripts/codex_cargo_env.sh` /
`scripts/codex_rust_guard_env.sh` creates a unique directory under
`build/codex-rust-timing-runs/run.XXXXXX` for each top-level command.
Separate concurrently running commands get separate ledgers rather than
overwriting each other's counters. Further Cargo stages within the same
top-level command keep the existing totals; the timer does not reset when
another crate or worker starts. `CODEX_ANDROID_RUSTC_GLOBAL_TIMING_DIR`
is exported to all compiler subprocesses.

The timestamp is local `HH:MM:SS`. Fields use compact duration units:
`h`, `m`, `s` (zero is `0s`). `slept` is time suspended or
waiting for memory; `this` is the still-uncommitted active time for
the reporting compiler process. The timer now also identifies its associated
compilation unit:

```text
15:36:02 ▶ compiling crate=codex_app_server file=app-server/src/lib.rs
15:37:03 kept=24m slept=0s lost=0s this=2m crate=codex_app_server file=app-server/src/lib.rs
15:37:25 ✓ finished crate=codex_app_server file=app-server/src/lib.rs
15:37:25 kept=26m22s slept=0s lost=0s this=0s last=codex_app_server file=app-server/src/lib.rs
15:37:26 ▶ compiling crate=core_test_support file=core/tests/common/lib.rs
```

The crate name and source entry file come directly from Rustc's arguments.
`file=` is the entry point (`lib.rs`, `main.rs`, or a test source), not
a live trace of internal modules. Rustc does not expose its current internal
module reliably, so the wrapper does not speculate. Start/finish/discard
transitions are emitted **for every compiler invocation**, without PIDs or
time deltas. They bypass the once-per-minute aggregate throttle so short
compilations remain visible. A `last=` field on an idle snapshot identifies
the source whose completion preceded it.

Managed Cargo builds also write a **separate temporary stage ledger** via
`CODEX_ANDROID_RUSTC_TIMING_DIR`. That ledger supplies per-stage ballast
autotuning and the final `⏱ Rust totals` line. Each ledger is protected
by its own lock. If the run ledger cannot be used, a warning is printed and
the wrapper falls back to stage-specific or local counters.

With parallel compiler invocations, the totals are summed
**compiler-process seconds** and may exceed the build's wall-clock elapsed time.
The in-progress field is local to the reporting rustc, not a build-wide total.
All values refer to time, not CPU instruction counts. The split is based on
whether the wrapper keeps an invocation's output; it cannot measure partially
reusable intermediate work within a terminated rustc process.

Test the shared timestamped guard-message throttle:

```sh
python3 -m unittest discover -s scripts/tests -p 'test_codex_rust_guard_output.py' -v
```

Run the deterministic timing tests on a Linux/Termux shell with Bash and flock:

```sh
bash scripts/tests/test_codex_rustc_timing.sh
```
