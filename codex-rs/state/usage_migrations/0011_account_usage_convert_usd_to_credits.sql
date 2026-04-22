UPDATE account_usage
SET
    total_usage_usd = total_usage_usd * 25.0,
    total_usage_usd_with_prewarm = total_usage_usd_with_prewarm * 25.0,
    last_reported_usage_usd = CASE
        WHEN last_reported_usage_usd IS NULL THEN NULL
        ELSE last_reported_usage_usd * 25.0
    END,
    usd_per_reported_percent = CASE
        WHEN usd_per_reported_percent IS NULL THEN NULL
        ELSE usd_per_reported_percent * 25.0
    END,
    cached_q_limit = CASE
        WHEN cached_q_limit IS NULL THEN NULL
        ELSE cached_q_limit * 25.0
    END;
