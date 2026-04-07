ALTER TABLE account_usage ADD COLUMN prewarm_sent_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage ADD COLUMN prewarm_recv_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage ADD COLUMN prewarm_sent_recv_bytes INTEGER NOT NULL DEFAULT 0;

ALTER TABLE account_usage ADD COLUMN last_snapshot_prewarm_sent_bytes INTEGER;
ALTER TABLE account_usage ADD COLUMN last_snapshot_prewarm_recv_bytes INTEGER;
ALTER TABLE account_usage ADD COLUMN last_snapshot_prewarm_sent_recv_bytes INTEGER;

ALTER TABLE account_usage ADD COLUMN window_start_prewarm_sent_bytes INTEGER;
ALTER TABLE account_usage ADD COLUMN window_start_prewarm_recv_bytes INTEGER;
ALTER TABLE account_usage ADD COLUMN window_start_prewarm_sent_recv_bytes INTEGER;

ALTER TABLE account_usage_samples ADD COLUMN delta_prewarm_sent_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage_samples ADD COLUMN delta_prewarm_recv_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage_samples ADD COLUMN delta_prewarm_sent_recv_bytes INTEGER NOT NULL DEFAULT 0;
