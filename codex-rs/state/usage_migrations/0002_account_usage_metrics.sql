ALTER TABLE account_usage ADD COLUMN context_total_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage ADD COLUMN min_total_cached_output_tokens INTEGER NOT NULL DEFAULT 0;

ALTER TABLE account_usage ADD COLUMN last_snapshot_input_tokens INTEGER;
ALTER TABLE account_usage ADD COLUMN last_snapshot_cached_input_tokens INTEGER;
ALTER TABLE account_usage ADD COLUMN last_snapshot_output_tokens INTEGER;
ALTER TABLE account_usage ADD COLUMN last_snapshot_context_total_tokens INTEGER;
ALTER TABLE account_usage ADD COLUMN last_snapshot_min_total_cached_output_tokens INTEGER;

ALTER TABLE account_usage ADD COLUMN window_start_input_tokens INTEGER;
ALTER TABLE account_usage ADD COLUMN window_start_cached_input_tokens INTEGER;
ALTER TABLE account_usage ADD COLUMN window_start_output_tokens INTEGER;
ALTER TABLE account_usage ADD COLUMN window_start_context_total_tokens INTEGER;
ALTER TABLE account_usage ADD COLUMN window_start_min_total_cached_output_tokens INTEGER;

ALTER TABLE account_usage_samples ADD COLUMN delta_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage_samples ADD COLUMN delta_cached_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage_samples ADD COLUMN delta_output_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage_samples ADD COLUMN delta_context_total_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage_samples ADD COLUMN delta_min_total_cached_output_tokens INTEGER NOT NULL DEFAULT 0;
