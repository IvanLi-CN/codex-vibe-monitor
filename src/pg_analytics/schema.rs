pub(crate) const PG_ANALYTICS_SCHEMA_NAME: &str = "cvm_analytics";

pub(crate) const PG_ANALYTICS_REQUIRED_RELATIONS: &[&str] = &[
    "cvm_analytics.analytics_facts",
    "cvm_analytics.analytics_parallel_work_keys",
    "cvm_analytics.analytics_daily_rollups",
    "cvm_analytics.analytics_hourly_rollups",
    "cvm_analytics.analytics_state",
    "cvm_analytics.analytics_projection_state",
    "cvm_analytics.analytics_projection_checkpoints",
    "cvm_analytics.analytics_projection_target_status",
];

pub(crate) const PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS: &[&str] = &[
    "cvm_analytics.analytics_replica_state",
    "cvm_analytics.analytics_replica_applied_changes",
];

pub(crate) const PG_ANALYTICS_REQUIRED_COLUMNS: &[(&str, &str)] = &[
    (
        "cvm_analytics.analytics_replica_state",
        "fact_date_semantics_version",
    ),
    ("cvm_analytics.analytics_replica_state", "owner_kind"),
    ("cvm_analytics.analytics_replica_state", "owner_epoch"),
    ("cvm_analytics.analytics_replica_state", "serving_fence"),
    (
        "cvm_analytics.analytics_replica_state",
        "serving_ack_change_id",
    ),
];

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
-- Projection readers use this tuple for high-watermark capture and keyset pages. Keep an
-- account-leading variant so scoped reads retain the same order without scanning other accounts.
CREATE INDEX IF NOT EXISTS analytics_facts_projection_cursor_idx
    ON cvm_analytics.analytics_facts (stats_date, occurred_at_epoch_ms, source, source_row_id);
CREATE INDEX IF NOT EXISTS analytics_facts_account_projection_cursor_idx
    ON cvm_analytics.analytics_facts (
        upstream_account_id, stats_date, occurred_at_epoch_ms, source, source_row_id
    );

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

-- Long-term maintenance/read-model projections. These tables mirror the SQLite
-- projection state so owner admission can require parity for every target.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_intervals (
    bucket_kind TEXT NOT NULL, bucket_date DATE NOT NULL, bucket_key TEXT NOT NULL,
    dimension TEXT NOT NULL, series_key TEXT NOT NULL, invocation_row_id BIGINT NOT NULL,
    interval_start_ms BIGINT NOT NULL, interval_end_ms BIGINT NOT NULL,
    PRIMARY KEY (bucket_kind, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms)
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_interval_state (
    invocation_row_id BIGINT PRIMARY KEY, model_series_key TEXT NOT NULL,
    upstream_series_key TEXT NOT NULL, interval_start_ms BIGINT NOT NULL, interval_end_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_interval_suppressions (
    invocation_row_id BIGINT NOT NULL, bucket_date DATE NOT NULL,
    PRIMARY KEY (invocation_row_id, bucket_date)
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_rebuild_members (
    rebuild_token TEXT NOT NULL, invocation_row_id BIGINT NOT NULL,
    PRIMARY KEY (rebuild_token, invocation_row_id)
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_dirty_buckets (
    bucket_date DATE PRIMARY KEY, repair_reason TEXT NOT NULL, generation BIGINT NOT NULL DEFAULT 1,
    queued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), next_attempt_at TIMESTAMPTZ, updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_bucket_state (
    bucket_date DATE PRIMARY KEY, interval_baseline_ready BOOLEAN NOT NULL DEFAULT FALSE,
    active_daily_backup_token TEXT, publication_token TEXT, publication_generation BIGINT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_daily_backups (
    rebuild_token TEXT NOT NULL, stats_date DATE NOT NULL, dimension TEXT NOT NULL, series_key TEXT NOT NULL,
    display_name TEXT NOT NULL, reasoning_effort TEXT NOT NULL DEFAULT '', calls BIGINT NOT NULL DEFAULT 0,
    token_total BIGINT NOT NULL DEFAULT 0, token_samples BIGINT NOT NULL DEFAULT 0,
    cost_total NUMERIC(20,8) NOT NULL DEFAULT 0, cost_samples BIGINT NOT NULL DEFAULT 0,
    usage_time_ms DOUBLE PRECISION NOT NULL DEFAULT 0, usage_time_samples BIGINT NOT NULL DEFAULT 0,
    wall_time_ms DOUBLE PRECISION NOT NULL DEFAULT 0, wall_time_samples BIGINT NOT NULL DEFAULT 0,
    output_tokens_total BIGINT NOT NULL DEFAULT 0, stream_duration_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    output_speed_samples BIGINT NOT NULL DEFAULT 0, first_byte_sum_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    first_byte_samples BIGINT NOT NULL DEFAULT 0, response_sum_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    response_samples BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (rebuild_token, stats_date, dimension, series_key)
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_date_publications (
    publication_token TEXT PRIMARY KEY, published BOOLEAN NOT NULL DEFAULT FALSE, updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_daily_backup_claims (
    bucket_date DATE PRIMARY KEY, rebuild_token TEXT NOT NULL, updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

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

-- V2 long-term projection lifecycle is kept separate from the replica ownership
-- state above.  This lets a stopped or rebuilding projection remain fail-closed
-- without rewriting the source facts or replica checkpoint.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_state (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    status TEXT NOT NULL DEFAULT 'preparing',
    statistics_start_date DATE,
    integrity_source_start_date DATE,
    integrity_source_pending_start_date DATE,
    processed_rows BIGINT NOT NULL DEFAULT 0,
    total_rows BIGINT NOT NULL DEFAULT 0,
    source_row_count BIGINT NOT NULL DEFAULT 0,
    source_digest TEXT,
    verified_row_count BIGINT NOT NULL DEFAULT 0,
    verified_digest TEXT,
    last_error_code TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO cvm_analytics.analytics_projection_state (id, status)
VALUES (1, 'preparing')
ON CONFLICT (id) DO NOTHING;

-- A checkpoint is scoped by consumer so rollup maintenance and interval/read
-- model consumers can advance independently while sharing the same generation.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_checkpoints (
    consumer TEXT PRIMARY KEY,
    cursor_row_id BIGINT NOT NULL DEFAULT 0,
    last_flush_at TIMESTAMPTZ,
    last_daily_verify_at TIMESTAMPTZ,
    daily_verify_pending BOOLEAN NOT NULL DEFAULT FALSE,
    daily_verify_bucket_date DATE,
    last_error_code TEXT,
    generation BIGINT NOT NULL DEFAULT 0 CHECK (generation >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS analytics_projection_checkpoints_generation_idx
    ON cvm_analytics.analytics_projection_checkpoints (generation, consumer);

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_target_status (
    target_kind TEXT PRIMARY KEY,
    ready BOOLEAN NOT NULL DEFAULT FALSE,
    generation BIGINT NOT NULL DEFAULT 0 CHECK (generation >= 0),
    source_row_count BIGINT NOT NULL DEFAULT 0,
    source_digest TEXT,
    last_error_code TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO cvm_analytics.analytics_projection_target_status (target_kind)
VALUES
    ('daily_rollups'),
    ('hourly_rollups'),
    ('interval_state'),
    ('read_model'),
    ('repair_state'),
    ('bucket_publication')
ON CONFLICT (target_kind) DO NOTHING;

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_replica_state (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    replica_id TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN (
        'unbootstrapped', 'bootstrapping', 'running', 'paused',
        'caught_up', 'invalidated', 'blocked', 'error'
    )),
    owner_kind TEXT NOT NULL DEFAULT 'sqlite' CHECK (owner_kind IN ('sqlite', 'postgres')),
    owner_epoch BIGINT NOT NULL DEFAULT 1 CHECK (owner_epoch > 0),
    serving_fence BIGINT NOT NULL DEFAULT 1 CHECK (serving_fence > 0),
    serving_ack_change_id BIGINT NOT NULL DEFAULT 0 CHECK (serving_ack_change_id >= 0),
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
    -- Legacy compatibility column; current writers never persist error bodies.
    last_error_detail TEXT CHECK (last_error_detail IS NULL),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Existing replica state is legacy until an explicit bootstrap rebuilds its facts.
ALTER TABLE cvm_analytics.analytics_replica_state
    ADD COLUMN IF NOT EXISTS fact_date_semantics_version BIGINT NOT NULL DEFAULT 1;
ALTER TABLE cvm_analytics.analytics_replica_state
    ADD COLUMN IF NOT EXISTS owner_kind TEXT NOT NULL DEFAULT 'sqlite';
ALTER TABLE cvm_analytics.analytics_replica_state
    ADD COLUMN IF NOT EXISTS owner_epoch BIGINT NOT NULL DEFAULT 1;
ALTER TABLE cvm_analytics.analytics_replica_state
    ADD COLUMN IF NOT EXISTS serving_fence BIGINT NOT NULL DEFAULT 1;
ALTER TABLE cvm_analytics.analytics_replica_state
    ADD COLUMN IF NOT EXISTS serving_ack_change_id BIGINT NOT NULL DEFAULT 0;

-- Older schemas may have accepted diagnostic bodies. Clear them and enforce the
-- current bounded-error-code contract before the replica can be used.
UPDATE cvm_analytics.analytics_replica_state
SET last_error_detail = NULL
WHERE last_error_detail IS NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_constraint AS constraint_row
        JOIN pg_catalog.pg_class AS relation ON relation.oid = constraint_row.conrelid
        JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
        WHERE namespace.nspname = 'cvm_analytics'
          AND relation.relname = 'analytics_replica_state'
          AND constraint_row.conname = 'analytics_replica_state_no_error_detail'
    ) THEN
        ALTER TABLE cvm_analytics.analytics_replica_state
            ADD CONSTRAINT analytics_replica_state_no_error_detail
            CHECK (last_error_detail IS NULL);
    END IF;
END
$$;

INSERT INTO cvm_analytics.analytics_replica_state (id, replica_id, status)
VALUES (1, 'pg-analytics', 'unbootstrapped')
ON CONFLICT (id) DO NOTHING;

-- Reapplying the owner contract is an explicit rollback fence until maintenance bootstrap
-- rebuilds the target with the current fact-date semantics. Move an eligible current reader
-- out of caught_up as well as fencing older programs that only inspect service_eligible.
UPDATE cvm_analytics.analytics_replica_state
SET service_eligible = FALSE,
    status = CASE WHEN status = 'caught_up' THEN 'bootstrapping' ELSE status END,
    updated_at = NOW()
WHERE id = 1;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_schema_includes_v2_long_term_projection_contract() {
        for relation in [
            "analytics_daily_rollups",
            "analytics_hourly_rollups",
            "analytics_projection_state",
            "analytics_projection_checkpoints",
        ] {
            assert!(
                PG_ANALYTICS_SCHEMA_SQL.contains(relation),
                "missing {relation}"
            );
        }
        for column in [
            "stats_date DATE NOT NULL",
            "bucket_start TIMESTAMPTZ NOT NULL",
            "reasoning_effort TEXT NOT NULL DEFAULT ''",
            "output_speed_samples BIGINT NOT NULL DEFAULT 0",
            "response_samples BIGINT NOT NULL DEFAULT 0",
            "integrity_source_start_date DATE",
            "daily_verify_pending BOOLEAN NOT NULL DEFAULT FALSE",
            "cursor_row_id BIGINT NOT NULL DEFAULT 0",
            "analytics_projection_target_status",
            "interval_state",
            "bucket_publication",
        ] {
            assert!(PG_ANALYTICS_SCHEMA_SQL.contains(column), "missing {column}");
        }
        assert!(
            PG_ANALYTICS_REQUIRED_RELATIONS.contains(&"cvm_analytics.analytics_projection_state")
        );
        assert!(
            PG_ANALYTICS_REQUIRED_RELATIONS
                .contains(&"cvm_analytics.analytics_projection_checkpoints")
        );
    }
}
