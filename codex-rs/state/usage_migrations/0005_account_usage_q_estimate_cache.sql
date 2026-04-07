ALTER TABLE account_usage ADD COLUMN cached_q_limit REAL;
ALTER TABLE account_usage ADD COLUMN cached_q_limit_sample_count INTEGER;
ALTER TABLE account_usage ADD COLUMN cached_q_limit_computed_at INTEGER;
ALTER TABLE account_usage ADD COLUMN cached_q_limit_for_updated_at INTEGER;
