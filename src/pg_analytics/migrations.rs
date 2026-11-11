use super::*;

// Published migration SQL is immutable. Keep V1 byte-for-byte identical to the
// deployed pre-ownership release; additive ownership columns belong to V2.
const MIGRATION_V1_SQL: &str = r#"
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
    -- Legacy compatibility column; current writers never persist error bodies.
    last_error_detail TEXT CHECK (last_error_detail IS NULL),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Existing replica state is legacy until an explicit bootstrap rebuilds its facts.
ALTER TABLE cvm_analytics.analytics_replica_state
    ADD COLUMN IF NOT EXISTS fact_date_semantics_version BIGINT NOT NULL DEFAULT 1;

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

const STOPPED_OWNERSHIP_V1_CHECKSUM: &str =
    "6336709fdec97a29130a6a8cf655c4d19bf882b9f53164e7a4cf9b515251d7af";

const MIGRATION_V2_SQL: &str = r#"
ALTER TABLE cvm_analytics.analytics_replica_state ADD COLUMN IF NOT EXISTS owner_kind TEXT NOT NULL DEFAULT 'sqlite';
ALTER TABLE cvm_analytics.analytics_replica_state ADD COLUMN IF NOT EXISTS owner_epoch BIGINT NOT NULL DEFAULT 1;
ALTER TABLE cvm_analytics.analytics_replica_state ADD COLUMN IF NOT EXISTS serving_fence BIGINT NOT NULL DEFAULT 1;
ALTER TABLE cvm_analytics.analytics_replica_state ADD COLUMN IF NOT EXISTS serving_ack_change_id BIGINT NOT NULL DEFAULT 0;
GRANT SELECT (owner_kind, owner_epoch, serving_fence, serving_ack_change_id)
    ON cvm_analytics.analytics_replica_state TO cvm_analytics_reader;
"#;

// V3 is the first additive long-term projection slice. The rollup tables mirror
// SQLite's long_term_usage_* shape so a future handoff can compare rows without
// a lossy translation. Projection lifecycle state is separate from replica
// ownership state, allowing a rebuild to remain fail-closed independently.
const MIGRATION_V3_SQL: &str = r#"
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

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_state (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    status TEXT NOT NULL DEFAULT 'preparing',
    statistics_start_date DATE,
    integrity_source_start_date DATE,
    integrity_source_pending_start_date DATE,
    processed_rows BIGINT NOT NULL DEFAULT 0,
    total_rows BIGINT NOT NULL DEFAULT 0,
    last_error_code TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
INSERT INTO cvm_analytics.analytics_projection_state (id, status)
VALUES (1, 'preparing')
ON CONFLICT (id) DO NOTHING;

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
"#;

// V4 records the source snapshot identity beside the projection lifecycle.  A ready
// projection is only serviceable after both row count and digest have been verified.
const MIGRATION_V4_SQL: &str = r#"
ALTER TABLE cvm_analytics.analytics_projection_state
    ADD COLUMN IF NOT EXISTS source_row_count BIGINT NOT NULL DEFAULT 0;
ALTER TABLE cvm_analytics.analytics_projection_state
    ADD COLUMN IF NOT EXISTS source_digest TEXT;
ALTER TABLE cvm_analytics.analytics_projection_state
    ADD COLUMN IF NOT EXISTS verified_row_count BIGINT NOT NULL DEFAULT 0;
ALTER TABLE cvm_analytics.analytics_projection_state
    ADD COLUMN IF NOT EXISTS verified_digest TEXT;
"#;

// V5 records parity independently for every serving projection target.  Rollups are
// currently materialized by the shadow copier; interval/read-model and repair targets
// stay fenced until their own writers publish a verified result.
const MIGRATION_V5_SQL: &str = r#"
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

GRANT SELECT (target_kind, ready, generation, source_row_count, source_digest,
              last_error_code, updated_at)
    ON cvm_analytics.analytics_projection_target_status TO cvm_analytics_reader;
GRANT SELECT, UPDATE
    ON cvm_analytics.analytics_projection_target_status TO cvm_analytics_maintenance;
GRANT SELECT
    ON cvm_analytics.analytics_daily_rollups,
       cvm_analytics.analytics_hourly_rollups,
       cvm_analytics.analytics_projection_state,
       cvm_analytics.analytics_projection_checkpoints
    TO cvm_analytics_reader;
GRANT SELECT, INSERT, UPDATE, DELETE
    ON cvm_analytics.analytics_daily_rollups,
       cvm_analytics.analytics_hourly_rollups,
       cvm_analytics.analytics_projection_state,
       cvm_analytics.analytics_projection_checkpoints
    TO cvm_analytics_maintenance;
"#;

fn checksum(sql: &str) -> String {
    format!("{:x}", Sha256::digest(sql.as_bytes()))
}

pub(super) async fn apply_ordered_migrations(
    connection: &mut sqlx::postgres::PgConnection,
) -> Result<()> {
    // Validate every recorded version before schema DDL. A failed check leaves both
    // the application schema and the migration ledger untouched.
    let mut tx = connection.begin().await?;
    sqlx::raw_sql(
        "CREATE SCHEMA IF NOT EXISTS cvm_analytics;
         CREATE TABLE IF NOT EXISTS cvm_analytics.schema_migrations (
           component TEXT NOT NULL, version BIGINT NOT NULL, checksum TEXT NOT NULL,
           state TEXT NOT NULL CHECK (state = 'applied'),
           min_reader_version BIGINT NOT NULL DEFAULT 1,
           requires_bootstrap BOOLEAN NOT NULL DEFAULT FALSE,
           error_code TEXT, applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
           PRIMARY KEY (component, version))",
    )
    .execute(&mut *tx)
    .await?;
    let recorded = sqlx::query_as::<_, (i64, String, String, i64)>(
        "SELECT version, checksum, state, min_reader_version FROM cvm_analytics.schema_migrations
         WHERE component = 'pg-analytics' ORDER BY version",
    )
    .fetch_all(&mut *tx)
    .await?;
    let migrations = [
        (1_i64, MIGRATION_V1_SQL),
        (2, MIGRATION_V2_SQL),
        (3, MIGRATION_V3_SQL),
        (4, MIGRATION_V4_SQL),
        (5, MIGRATION_V5_SQL),
    ];
    for (version, recorded_checksum, state, min_reader) in &recorded {
        anyhow::ensure!(
            state == "applied" && *min_reader <= 5,
            "PostgreSQL analytics migration is incomplete or requires a newer reader at version {version}"
        );
        let sql = migrations
            .iter()
            .find(|(known, _)| known == version)
            .map(|(_, sql)| *sql)
            .context("PostgreSQL analytics schema is newer than this reader")?;
        // The first ownership release incorrectly recorded its additive schema as
        // V1. Recognize that exact published variant without rewriting its history.
        let stopped_ownership_v1 =
            *version == 1 && recorded_checksum == STOPPED_OWNERSHIP_V1_CHECKSUM;
        anyhow::ensure!(
            *recorded_checksum == checksum(sql) || stopped_ownership_v1,
            "PostgreSQL analytics migration checksum mismatch at version {version}"
        );
    }
    anyhow::ensure!(
        recorded
            .iter()
            .enumerate()
            .all(|(index, (version, _, _, _))| *version == index as i64 + 1),
        "PostgreSQL analytics migration history has a gap"
    );
    for (version, sql) in migrations {
        if recorded
            .iter()
            .any(|(applied, _, _, _)| *applied == version)
        {
            continue;
        }
        sqlx::raw_sql(sql).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO cvm_analytics.schema_migrations
             (component, version, checksum, state, min_reader_version, requires_bootstrap)
             VALUES ('pg-analytics', $1, $2, 'applied', $3, $4)",
        )
        .bind(version)
        .bind(checksum(sql))
        .bind(version)
        .bind(version == 1)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v3_adds_shape_compatible_rollups_and_projection_progress() {
        for table in [
            "analytics_daily_rollups",
            "analytics_hourly_rollups",
            "analytics_projection_state",
            "analytics_projection_checkpoints",
        ] {
            assert!(
                MIGRATION_V3_SQL.contains(table),
                "V3 must provision {table}"
            );
        }
        for column in [
            "calls BIGINT NOT NULL DEFAULT 0",
            "token_total BIGINT NOT NULL DEFAULT 0",
            "token_samples BIGINT NOT NULL DEFAULT 0",
            "cost_total NUMERIC(20, 8) NOT NULL DEFAULT 0",
            "usage_time_ms DOUBLE PRECISION NOT NULL DEFAULT 0",
            "wall_time_ms DOUBLE PRECISION NOT NULL DEFAULT 0",
            "output_tokens_total BIGINT NOT NULL DEFAULT 0",
            "stream_duration_ms DOUBLE PRECISION NOT NULL DEFAULT 0",
            "first_byte_sum_ms DOUBLE PRECISION NOT NULL DEFAULT 0",
            "response_sum_ms DOUBLE PRECISION NOT NULL DEFAULT 0",
        ] {
            assert!(
                MIGRATION_V3_SQL.contains(column),
                "missing rollup column {column}"
            );
        }
        assert!(MIGRATION_V3_SQL.contains("ON CONFLICT (id) DO NOTHING"));
        assert!(MIGRATION_V3_SQL.contains("generation BIGINT NOT NULL DEFAULT 0"));
        assert!(!MIGRATION_V3_SQL.contains("DROP TABLE"));
    }

    #[test]
    fn migration_checksums_are_distinct_and_v3_is_additive() {
        assert_ne!(checksum(MIGRATION_V1_SQL), checksum(MIGRATION_V2_SQL));
        assert_ne!(checksum(MIGRATION_V2_SQL), checksum(MIGRATION_V3_SQL));
        assert!(MIGRATION_V3_SQL.trim_start().starts_with("CREATE TABLE"));
    }

    #[test]
    fn v4_adds_projection_verification_metadata() {
        for column in [
            "source_row_count BIGINT NOT NULL DEFAULT 0",
            "source_digest TEXT",
            "verified_row_count BIGINT NOT NULL DEFAULT 0",
            "verified_digest TEXT",
        ] {
            assert!(MIGRATION_V4_SQL.contains(column), "missing {column}");
        }
        assert!(!MIGRATION_V4_SQL.contains("DROP TABLE"));
    }

    #[test]
    fn v5_adds_explicit_projection_target_parity() {
        for target in [
            "daily_rollups",
            "hourly_rollups",
            "interval_state",
            "read_model",
            "repair_state",
            "bucket_publication",
        ] {
            assert!(MIGRATION_V5_SQL.contains(target), "missing target {target}");
        }
        assert!(MIGRATION_V5_SQL.contains("ready BOOLEAN NOT NULL DEFAULT FALSE"));
        assert!(!MIGRATION_V5_SQL.contains("DROP TABLE"));
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL *_fixture database and explicit schema reset authorization"]
    async fn postgres_ordered_migrations_upgrade_and_reentry() -> Result<()> {
        anyhow::ensure!(
            env::var("PG_ANALYTICS_TEST_ALLOW_SCHEMA_RESET").as_deref() == Ok("1"),
            "fixture reset authorization required"
        );
        let mut connection =
            sqlx::postgres::PgConnection::connect(&env::var("PG_ANALYTICS_TEST_OWNER_URL")?)
                .await?;
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut connection)
            .await?;
        anyhow::ensure!(
            database.ends_with("_fixture"),
            "isolated fixture database required"
        );

        let result = async {
            for source in ["empty", "deployed_v1", "stopped_ownership_v1"] {
                sqlx::raw_sql("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
                    .execute(&mut connection).await?;
                if source != "empty" {
                    let source_sql = if source == "deployed_v1" { MIGRATION_V1_SQL } else { PG_ANALYTICS_SCHEMA_SQL };
                    sqlx::raw_sql(source_sql).execute(&mut connection).await?;
                    sqlx::raw_sql("CREATE TABLE cvm_analytics.schema_migrations (
                        component TEXT NOT NULL, version BIGINT NOT NULL, checksum TEXT NOT NULL,
                        state TEXT NOT NULL, min_reader_version BIGINT NOT NULL DEFAULT 1,
                        requires_bootstrap BOOLEAN NOT NULL DEFAULT FALSE, error_code TEXT,
                        applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), PRIMARY KEY(component, version))")
                        .execute(&mut connection).await?;
                    let recorded_checksum = if source == "stopped_ownership_v1" {
                        STOPPED_OWNERSHIP_V1_CHECKSUM.to_string()
                    } else {
                        checksum(source_sql)
                    };
                    sqlx::query("INSERT INTO cvm_analytics.schema_migrations(component,version,checksum,state) VALUES ('pg-analytics',1,$1,'applied')")
                        .bind(recorded_checksum).execute(&mut connection).await?;
                }
                apply_ordered_migrations(&mut connection).await?;
                let versions: Vec<i64> = sqlx::query_scalar("SELECT version FROM cvm_analytics.schema_migrations ORDER BY version")
                    .fetch_all(&mut connection).await?;
                assert_eq!(versions, [1, 2, 3, 4, 5], "source={source}");
                sqlx::query("UPDATE cvm_analytics.analytics_replica_state SET status='caught_up', consumer_checkpoint=17, source_acknowledged_through=17, serving_ack_change_id=17 WHERE id=1")
                    .execute(&mut connection).await?;
                let before: serde_json::Value = sqlx::query_scalar("SELECT to_jsonb(s) FROM cvm_analytics.analytics_replica_state s WHERE id=1")
                    .fetch_one(&mut connection).await?;
                apply_ordered_migrations(&mut connection).await?;
                let after: serde_json::Value = sqlx::query_scalar("SELECT to_jsonb(s) FROM cvm_analytics.analytics_replica_state s WHERE id=1")
                    .fetch_one(&mut connection).await?;
                assert_eq!(before, after, "restart must preserve replica state: {source}");
                // A rejected migration must leave the target and recorded history unchanged.
                sqlx::query("UPDATE cvm_analytics.schema_migrations SET checksum='invalid' WHERE version=1")
                    .execute(&mut connection).await?;
                assert!(apply_ordered_migrations(&mut connection).await.is_err());
                let rejected: serde_json::Value = sqlx::query_scalar("SELECT to_jsonb(s) FROM cvm_analytics.analytics_replica_state s WHERE id=1")
                    .fetch_one(&mut connection).await?;
                assert_eq!(before, rejected, "checksum rejection must not fence or mutate: {source}");
            }
            Ok::<(), anyhow::Error>(())
        }.await;
        let cleanup = sqlx::raw_sql("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
            .execute(&mut connection)
            .await;
        result?;
        cleanup?;
        Ok(())
    }
}
