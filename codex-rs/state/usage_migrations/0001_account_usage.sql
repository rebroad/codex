CREATE TABLE account_usage (
    account_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cached_input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_output_tokens INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    last_backend_limit_id TEXT,
    last_backend_limit_name TEXT,
    last_backend_used_percent REAL,
    last_snapshot_total_tokens INTEGER,
    last_snapshot_percent_int INTEGER,
    window_start_percent_int INTEGER,
    window_start_total_tokens INTEGER,
    last_backend_resets_at INTEGER,
    last_backend_window_minutes INTEGER,
    last_backend_seen_at INTEGER,
    PRIMARY KEY (account_id, provider)
);

CREATE TABLE account_usage_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    observed_at INTEGER NOT NULL,
    start_percent_int INTEGER NOT NULL,
    end_percent_int INTEGER NOT NULL,
    delta_percent_int INTEGER NOT NULL,
    delta_tokens INTEGER NOT NULL,
    window_minutes INTEGER,
    resets_at INTEGER
);

CREATE INDEX idx_account_usage_samples_account
    ON account_usage_samples(account_id, provider, observed_at);
