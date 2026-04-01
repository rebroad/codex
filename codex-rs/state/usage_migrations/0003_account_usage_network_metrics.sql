ALTER TABLE account_usage ADD COLUMN sent_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage ADD COLUMN recv_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage ADD COLUMN sent_recv_bytes INTEGER NOT NULL DEFAULT 0;

ALTER TABLE account_usage ADD COLUMN last_snapshot_sent_bytes INTEGER;
ALTER TABLE account_usage ADD COLUMN last_snapshot_recv_bytes INTEGER;
ALTER TABLE account_usage ADD COLUMN last_snapshot_sent_recv_bytes INTEGER;

ALTER TABLE account_usage ADD COLUMN window_start_sent_bytes INTEGER;
ALTER TABLE account_usage ADD COLUMN window_start_recv_bytes INTEGER;
ALTER TABLE account_usage ADD COLUMN window_start_sent_recv_bytes INTEGER;

ALTER TABLE account_usage_samples ADD COLUMN delta_sent_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage_samples ADD COLUMN delta_recv_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE account_usage_samples ADD COLUMN delta_sent_recv_bytes INTEGER NOT NULL DEFAULT 0;
