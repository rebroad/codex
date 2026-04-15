ALTER TABLE account_usage ADD COLUMN total_usage_usd_with_prewarm REAL NOT NULL DEFAULT 0;

UPDATE account_usage
SET
    total_usage_usd_with_prewarm = total_usage_usd / 1000000.0,
    total_usage_usd = total_usage_usd / 1000000.0;
