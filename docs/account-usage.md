# Account Usage Tracking

This document describes how Codex tracks local account token usage and how it is correlated with backend rate-limit snapshots. The tracker is intentionally stored outside the upstream state database so it does not introduce incompatible migrations.

## Storage Location

Usage data is stored in a dedicated SQLite database:

- Path: `config.sqlite_home/usage_1.sqlite`
- Default `sqlite_home` resolution:
  - If `CODEX_SQLITE_HOME` is set, it is used.
  - Otherwise it falls back to `CODEX_HOME`.

This keeps usage tracking isolated from the upstream `state_*.sqlite` files.

Usage events are also appended to per-account log files under a usage-log directory:

- Default path: `$HOME/.codex/log/usage-<email>.log`
- Override: `CODEX_USAGE_LOG_DIR=/path/to/logdir`
- Each line includes timestamp, pid, percent/sample metadata, and event details.
- The account email is no longer duplicated on each line because it is encoded in the filename.

Threshold-crossing events are also written to shared usage-limit logs:

- `$HOME/.codex/log/usage-limit-100.log` (or `CODEX_USAGE_LOG_DIR/usage-limit-100.log`)
- `$HOME/.codex/log/usage-limit-101.log` (or `CODEX_USAGE_LOG_DIR/usage-limit-101.log`)

Each threshold line records:

- `account`
- `input`
- `cached_input`
- `output`
- `recv_bytes`
- `sent_bytes`
- `recv_bytes_including_warmups`
- `sent_bytes_including_warmups`

## Compatibility Notes

Earlier iterations of the tracker stored usage tables in the upstream state DB. Those tables are no longer used. If you previously ran a build that added those migrations, you may need to reset the state DB (or point `CODEX_SQLITE_HOME` to a fresh directory) to clear SQLx migration mismatches. This does not affect the usage database described above.

## Schema

The usage database has two tables:

- `account_usage` (live totals and latest backend snapshot metadata)
- `account_usage_samples` (incremental samples used to estimate token allowance)

“Account+provider” means the account identifier (account id or email-based key)
paired with the configured model provider id (for example `openai`).

### `account_usage`

Used for: live per-account totals and the most recent backend snapshot metadata.

Fields:

- `account_id`
- `provider`
- `total_tokens`
- `input_tokens`
- `cached_input_tokens`
- `output_tokens`
- `reasoning_output_tokens`
- `sent_bytes`
- `recv_bytes`
- `sent_recv_bytes`
- `updated_at`
- `last_backend_limit_id`
- `last_backend_limit_name`
- `last_backend_used_percent`
- `last_snapshot_total_tokens` (local total captured at snapshot time)
- `last_snapshot_sent_bytes`
- `last_snapshot_recv_bytes`
- `last_snapshot_sent_recv_bytes`
- `last_snapshot_percent_int`
- `window_start_percent_int` (latest backend integer anchor used for prediction)
- `window_start_total_tokens` (local token total at latest backend anchor)
- `window_start_sent_bytes`
- `window_start_recv_bytes`
- `window_start_sent_recv_bytes`
- `last_backend_resets_at`
- `last_backend_window_minutes`
- `last_backend_seen_at`

### `account_usage_samples`

Used for: estimating the token allowance without waiting for a full reset. Reset events are represented by the final sample recorded before the reset.

Samples are recorded whenever `used_percent` crosses an integer percentage boundary (for example 1% -> 2%). Each sample stores the local token delta observed since the last sample, along with the percent delta. When a reset is observed, we also record a summary sample for the full observed window so far.

If `used_percent` decreases without a reset (for example a refund after an unauthorized model attempt), we treat it as a refund event and subtract the most recent in-memory token delta from the live totals. No sample is recorded for the refund itself. This delta is stored in memory only, so refunds observed after a restart will be ignored (and logged).

Fields:

- `account_id`
- `provider`
- `observed_at`
- `start_percent_int`
- `end_percent_int`
- `delta_percent_int`
- `delta_tokens`
- `delta_sent_bytes`
- `delta_recv_bytes`
- `delta_sent_recv_bytes`
- `window_minutes`
- `resets_at`

Retention:

- Samples are pruned at reset time.

## Correlating With Backend Usage

The backend provides `used_percent` for the window. For each tracked metric, Codex derives an estimated allowance by blending:

- a sampled estimate from `account_usage_samples` deltas (`delta_metric / delta_percent`), and
- a cumulative estimate from running totals (`current_metric / smoothed_backend_percent`).

The sampled estimate receives more weight as sample count increases (up to a configurable cap). The cumulative estimate keeps results stable earlier in the window when sample volume is low.

`usage_pct` prediction is anchored to the latest backend snapshot percent and the local token totals observed at that same snapshot. As local token totals increase between backend snapshots, predicted percent advances using the estimated allowance:

```
estimated_limit = sum(delta_tokens) / (sum(delta_percent_int) / 100)
estimated_percent = backend_anchor_percent + ((local_total_tokens - anchor_total_tokens) / estimated_limit * 100)
```

This estimate is displayed in `/status` when the correlation is available.
`usage_pct` is hidden until a minimum sample count is reached.
Before the backend reaches 100%, `usage_pct` values are capped by configuration; after backend `used_percent >= 100`, values are allowed to grow beyond 100.

Current `usage_pct` log order is:

- `q`: composite calibrated metric (`output + 0.006*input + 0.003*cached_input`)
- `w`: weighted bytes including prewarm traffic (`a*(sent+prewarm_sent) + (1-a)*(recv+prewarm_recv)`)
- `p`: weighted bytes excluding prewarm traffic (`a*sent + (1-a)*recv`)
  where `a` is refit from recent samples to stay aligned with stabilized backend percent.
- `b`: blended total tokens
- `c`: cached input
- `o`: output
- `x`: context total
- `m`: `min(total, cached_input + output)`
- `n`: `min(input, cached_input) + output`
- `s`: sent bytes
- `r`: recv bytes
- `z`: sent+recv bytes

## Reset Behavior

A reset is recorded when:

- The backend snapshot is newer, and
- `used_percent` drops to zero, or the window metadata changes (`resets_at` or `window_minutes`).

When a reset is detected:

- The live `account_usage` totals are reset to zero.

## Querying Usage

Example queries:

```sql
SELECT * FROM account_usage;
```

```sql
SELECT account_id, total_tokens, last_backend_used_percent
FROM account_usage
WHERE provider = 'openai';
```

## Clearing Usage Data

Use the CLI command below to remove local usage tracking rows:

```bash
codex usage clear
```

This targets the current resolved account identity for the active provider.
To clear all locally tracked accounts for the active provider:

```bash
codex usage clear --all-accounts
```
