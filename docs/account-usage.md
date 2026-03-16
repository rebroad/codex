# Account Usage Tracking

This document describes how Codex tracks local account token usage and how it is correlated with backend rate-limit snapshots. The tracker is intentionally stored outside the upstream state database so it does not introduce incompatible migrations.

## Storage Location

Usage data is stored in a dedicated SQLite database:

- Path: `config.sqlite_home/usage_1.sqlite`
- Default `sqlite_home` resolution:
  - If `CODEX_SQLITE_HOME` is set, it is used.
  - Otherwise it falls back to `CODEX_HOME`.

This keeps usage tracking isolated from the upstream `state_*.sqlite` files.

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
- `updated_at`
- `last_backend_limit_id`
- `last_backend_limit_name`
- `last_backend_used_percent`
- `last_snapshot_total_tokens` (local total captured at snapshot time)
- `last_snapshot_percent_int`
- `window_start_percent_int`
- `window_start_total_tokens`
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
- `window_minutes`
- `resets_at`

Retention:
- Samples are pruned at reset time.

## Correlating With Backend Usage

The backend provides `used_percent` for the window. When one or more samples exist for the current `resets_at`, we estimate the token allowance using the total positive percent deltas and total token deltas, then compute an approximate percent for the locally tracked total:

```
estimated_limit = sum(delta_tokens) / (sum(delta_percent_int) / 100)
estimated_percent = local_total_tokens / estimated_limit * 100
```

This estimate is displayed in `/status` when the correlation is available.

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
