ALTER TABLE account_usage
    ADD COLUMN last_reported_usage_usd REAL;

ALTER TABLE account_usage
    ADD COLUMN usd_per_reported_percent REAL;
