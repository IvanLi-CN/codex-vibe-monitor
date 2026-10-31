pub(crate) const PG_ANALYTICS_SCHEMA_NAME: &str = "cvm_analytics";

pub(crate) const PG_ANALYTICS_REQUIRED_RELATIONS: &[&str] = &[
    "cvm_analytics.analytics_facts",
    "cvm_analytics.analytics_parallel_work_keys",
    "cvm_analytics.analytics_daily_rollups",
    "cvm_analytics.analytics_hourly_rollups",
    "cvm_analytics.analytics_state",
];

pub(crate) const PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS: &[&str] = &[
    "cvm_analytics.analytics_replica_state",
    "cvm_analytics.analytics_replica_applied_changes",
];

pub(crate) const PG_ANALYTICS_REQUIRED_COLUMNS: &[(&str, &str)] = &[(
    "cvm_analytics.analytics_replica_state",
    "fact_date_semantics_version",
)];

// Provisioning is intentionally separate from application startup. The runtime only verifies
// these relations and never creates or mutates the analytics schema implicitly.
pub(crate) const PG_ANALYTICS_SCHEMA_SQL: &str = r#"
CREATE SCHEMA IF NOT EXISTS cvm_analytics;

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_facts (
    fact_id BIGSERIAL PRIMARY KEY,
    fact_version SMALLINT NOT NULL DEFAULT 1,
    source TEXT NOT NULL,
    source_row_id BIGINT NOT NULL,
    invoke_id TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    occurred_at_epoch_ms BIGINT NOT NULL,
    stats_date DATE NOT NULL,
    model TEXT,
    request_model TEXT,
    response_model TEXT,
    reasoning_effort TEXT,
    upstream_account_id BIGINT,
    upstream_account_name TEXT,
    status TEXT,
    success_like BOOLEAN NOT NULL,
    terminal BOOLEAN NOT NULL,
    failure_kind TEXT,
    failure_class TEXT,
    is_actionable BOOLEAN,
    input_tokens BIGINT,
    output_tokens BIGINT,
    cache_input_tokens BIGINT,
    reasoning_tokens BIGINT,
    total_tokens BIGINT,
    cost NUMERIC(20, 8),
    cost_input NUMERIC(20, 8),
    cost_cache_write NUMERIC(20, 8),
    cost_cache_read NUMERIC(20, 8),
    cost_output NUMERIC(20, 8),
    cost_reasoning NUMERIC(20, 8),
    t_total_ms DOUBLE PRECISION,
    t_req_read_ms DOUBLE PRECISION,
    t_req_parse_ms DOUBLE PRECISION,
    t_upstream_connect_ms DOUBLE PRECISION,
    t_upstream_ttfb_ms DOUBLE PRECISION,
    t_upstream_stream_ms DOUBLE PRECISION,
    first_token_ms DOUBLE PRECISION,
    t_resp_parse_ms DOUBLE PRECISION,
    UNIQUE (source, source_row_id)
);

CREATE INDEX IF NOT EXISTS analytics_facts_stats_date_idx
    ON cvm_analytics.analytics_facts (stats_date);
CREATE INDEX IF NOT EXISTS analytics_facts_model_date_idx
    ON cvm_analytics.analytics_facts (model, stats_date);
CREATE INDEX IF NOT EXISTS analytics_facts_upstream_date_idx
    ON cvm_analytics.analytics_facts (upstream_account_id, stats_date);
CREATE INDEX IF NOT EXISTS analytics_facts_upstream_occurred_idx
    ON cvm_analytics.analytics_facts (upstream_account_id, occurred_at_epoch_ms, source_row_id);

-- Parallel-work membership uses a one-way key digest. Raw prompt-cache keys never cross the
-- analytics storage boundary, while the source row identity keeps corrections idempotent.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_parallel_work_keys (
    source TEXT NOT NULL,
    source_row_id BIGINT NOT NULL,
    occurred_at_epoch_ms BIGINT NOT NULL,
    key_digest TEXT NOT NULL,
    upstream_account_id BIGINT,
    PRIMARY KEY (source, source_row_id)
);

CREATE INDEX IF NOT EXISTS analytics_parallel_work_keys_window_idx
    ON cvm_analytics.analytics_parallel_work_keys (
        occurred_at_epoch_ms, source, upstream_account_id, key_digest
    );

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_daily_rollups (
    stats_date DATE NOT NULL,
    dimension TEXT NOT NULL,
    series_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    reasoning_effort TEXT NOT NULL DEFAULT '',
    calls BIGINT NOT NULL DEFAULT 0,
    token_total BIGINT NOT NULL DEFAULT 0,
    token_samples BIGINT NOT NULL DEFAULT 0,
    cost_total NUMERIC(20, 8) NOT NULL DEFAULT 0,
    cost_samples BIGINT NOT NULL DEFAULT 0,
    usage_time_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    usage_time_samples BIGINT NOT NULL DEFAULT 0,
    wall_time_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    wall_time_samples BIGINT NOT NULL DEFAULT 0,
    output_tokens_total BIGINT NOT NULL DEFAULT 0,
    stream_duration_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    output_speed_samples BIGINT NOT NULL DEFAULT 0,
    first_byte_sum_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    first_byte_samples BIGINT NOT NULL DEFAULT 0,
    response_sum_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    response_samples BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (stats_date, dimension, series_key)
);

CREATE INDEX IF NOT EXISTS analytics_daily_rollups_dimension_date_idx
    ON cvm_analytics.analytics_daily_rollups (dimension, stats_date);

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_hourly_rollups (
    bucket_start TIMESTAMPTZ NOT NULL,
    dimension TEXT NOT NULL,
    series_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    reasoning_effort TEXT NOT NULL DEFAULT '',
    calls BIGINT NOT NULL DEFAULT 0,
    token_total BIGINT NOT NULL DEFAULT 0,
    token_samples BIGINT NOT NULL DEFAULT 0,
    cost_total NUMERIC(20, 8) NOT NULL DEFAULT 0,
    cost_samples BIGINT NOT NULL DEFAULT 0,
    usage_time_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    usage_time_samples BIGINT NOT NULL DEFAULT 0,
    wall_time_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    wall_time_samples BIGINT NOT NULL DEFAULT 0,
    output_tokens_total BIGINT NOT NULL DEFAULT 0,
    stream_duration_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    output_speed_samples BIGINT NOT NULL DEFAULT 0,
    first_byte_sum_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    first_byte_samples BIGINT NOT NULL DEFAULT 0,
    response_sum_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    response_samples BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (bucket_start, dimension, series_key)
);

CREATE INDEX IF NOT EXISTS analytics_hourly_rollups_dimension_bucket_idx
    ON cvm_analytics.analytics_hourly_rollups (dimension, bucket_start);

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_state (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    status TEXT NOT NULL DEFAULT 'preparing',
    statistics_start_date DATE,
    processed_rows BIGINT NOT NULL DEFAULT 0,
    total_rows BIGINT NOT NULL DEFAULT 0,
    last_error_code TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO cvm_analytics.analytics_state (id, status)
VALUES (1, 'preparing')
ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_replica_state (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    replica_id TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN (
        'unbootstrapped', 'bootstrapping', 'running', 'paused',
        'caught_up', 'invalidated', 'blocked', 'error'
    )),
    fact_date_semantics_version BIGINT NOT NULL DEFAULT 2,
    generation BIGINT NOT NULL DEFAULT 0,
    baseline_generation BIGINT NOT NULL DEFAULT 0,
    baseline_change_id BIGINT NOT NULL DEFAULT 0,
    coverage_generation BIGINT NOT NULL DEFAULT 0,
    coverage_revision BIGINT NOT NULL DEFAULT 0,
    coverage_watermark BIGINT NOT NULL DEFAULT 0,
    consumer_checkpoint BIGINT NOT NULL DEFAULT 0,
    source_acknowledged_through BIGINT NOT NULL DEFAULT 0,
    service_eligible BOOLEAN NOT NULL DEFAULT FALSE,
    last_error_code TEXT,
    last_error_detail TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Existing replica state is legacy until an explicit bootstrap rebuilds its facts.
ALTER TABLE cvm_analytics.analytics_replica_state
    ADD COLUMN IF NOT EXISTS fact_date_semantics_version BIGINT NOT NULL DEFAULT 1;

INSERT INTO cvm_analytics.analytics_replica_state (id, replica_id, status)
VALUES (1, 'pg-analytics', 'unbootstrapped')
ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_replica_applied_changes (
    change_id BIGINT PRIMARY KEY,
    generation BIGINT NOT NULL,
    coverage_revision BIGINT NOT NULL,
    event_kind TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    source TEXT NOT NULL,
    source_row_id BIGINT,
    entity_key TEXT NOT NULL,
    fact_bytes INTEGER NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS analytics_replica_applied_generation_idx
    ON cvm_analytics.analytics_replica_applied_changes (generation, change_id);
"#;
