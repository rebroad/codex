# Android / Termux memory-kill evidence and Codex build guard

Captured from the connected Samsung SM-F766B on 2026-10-24 after reboot. Device
reported Android 16 / API 36, security patch 2026-08-05. Commands were run through
ADB; the Codex wrapper itself does not call ADB or `dumpsys`.

## Observed Termux exits

`dumpsys activity exit-info com.termux` retained 16 exits. Eight were reported
as `LOW_MEMORY`, all with unknown subreason and no trace or description:

| Timestamp | Importance | RSS |
| --- | ---: | ---: |
| 2026-10-24 07:48:42 | 125 | 59 MB |
| 2026-10-23 22:42:28 | 125 | 62 MB |
| 2026-10-23 18:25:42 | 125 | 62 MB |
| 2026-10-22 23:20:13 | 125 | 54 MB |
| 2026-10-22 22:29:43 | 125 | 67 MB |
| 2026-10-19 22:13:13 | 400 | 118 MB |
| 2026-10-06 20:43:46 | 100 | 72 MB |
| 2026-10-06 18:34:03 | 100 | 104 MB |

The Android importance values indicate foreground-service state at 125,
foreground at 100, and background at 400. Seven of these exits happened while
Termux was foreground or running its foreground service; one happened while it
was background. Exit info does not identify the pressure signal that caused
these kills, nor does it prove a Rust process caused them.

The other retained exits were two `TOO MANY EMPTY PROCS` kills (importance 400,
RSS 0), three package-update stops, and three user-requested force-stops. The
empty-process kills are process trimming and are not addressed by a compiler
pressure guard.

## Observed Android pressure policy

`dumpsys activity lmk` reported Samsung's live LMKD configuration:

- Free-swap starvation threshold: 40% of total swap. `swap_util_max=100`, so
  there is no lower independent swap-utilization cap in this configuration.
- PSI medium: stall type 0, threshold 70 ms.
- PSI critical: stall type 1, threshold 120 ms.
- PSI super-critical: stall type 1, threshold 700 ms.
- Thrashing limit: 100%, with 10% decay and 300% critical threshold.
- Background-process keeping policy enabled.

The AOSP LMKD implementation describes PSI thresholds in milliseconds within a
one-second window; its PSI monitor window is 1000 ms. Mapping Samsung's type 0
to partial (`some`) stalls and type 1 to complete (`full`) stalls follows the
LMKD PSI model. Samsung's fields are vendor configuration, so this mapping is
an inference rather than an exposed contract. See the [AOSP LMKD guide](https://source.android.com/docs/core/perf/lmkd)
and [AOSP LMKD source](https://android.googlesource.com/platform/system/memory/lmkd/+/refs/heads/main/lmkd.cpp).

The current LMKD statistics snapshot showed 37 kills, all at oom-adj scores
901–998, all during medium pressure, and all recorded with reason `low_mem`.
There were no recorded critical kills or kills with the low-swap reason in that
snapshot. The stats do not name victims, and they do not connect those 37 kills
to the eight Termux exit-info records. This means the swap percentage alone
does not describe the Termux failures.

At the same inspection, physical RAM was 11.0 GiB, `MemAvailable` was 6.4 GiB,
total swap was 16.0 GiB, and free swap was 14.1 GiB. The Termux process was
running with oom adjustment 200. These are a point-in-time snapshot, not the
conditions at the earlier exits. The retained logcat did not contain historical
LMKD victim records for those exits.

At inspection time, Termux had a foreground service, but its live
`oom_score_adj` was 200. This value is dynamic and cannot be forced to zero by
the unprivileged app. `/proc/meminfo` and `/proc/pressure/memory` were readable
from the ADB shell; no runtime ADB access is assumed by the build wrapper.

## Codex Rust compiler buffer

The native Android Cargo environment enables the guard in
`scripts/codex_rustc_oom_wrapper.sh`. It samples only local procfs data and stops
the Rust compiler process group when any of these buffers is reached:

- Free swap at or below 50% of total swap. This is 10 percentage points above
  the observed 40% Android low-swap threshold (about 1.6 GiB of extra free swap
  on the observed 16 GiB swap configuration).
- `MemAvailable` at or below 20% of physical RAM. This is a conservative
  reserve selected because the exact Samsung low-memory watermark that caused
  the Termux exits is not exposed in exit info.
- PSI `some` stalls at or above 35 ms in the rolling one-second sample, half of
  the observed 70 ms medium threshold.
- PSI `full` stalls at or above 60 ms in the rolling one-second sample, half of
  the observed 120 ms critical threshold.

The half-threshold PSI buffers give the wrapper a chance to stop compilation
before Android reaches the observed medium or critical trigger. Rust compiler
work runs in a separate session/process group; on threshold the wrapper sends
TERM, then KILL after two seconds, and returns status 75 so Cargo reports a
failed build. The OOM adjustment remains 800. Android's own medium-pressure
policy should also prefer these compiler processes over Termux at score 200.

When sccache is active, Cargo uses sccache client-side mode so the compiler
stays in the supervised process group instead of running in the long-lived
sccache server. The guard covers Rust compiler invocations launched through
`RUSTC_WRAPPER`; it does not supervise unrelated processes or direct C compiler
calls made by build scripts. This mode is supported by the installed sccache
0.18.0 and documented in [sccache configuration](https://github.com/mozilla/sccache/blob/main/docs/Configuration.md).
`CARGO_BUILD_JOBS=1` remains the Android default.

These limits are calibrated to the captured phone configuration. They are
overridable through the `CODEX_ANDROID_*` environment values emitted by
`codex_cargo_env.sh`. If Android policy or swap sizing changes, recapture the
LMKD dump and update the baseline values before reducing the buffers.
