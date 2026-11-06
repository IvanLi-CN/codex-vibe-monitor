-- The ingest receipt is additive to the module registry. It is applied by the schema owner
-- before the maintenance consumer starts; the consumer itself never creates PostgreSQL objects.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_module_ingest_receipts (
    consumer_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    generation BIGINT NOT NULL CHECK (generation >= 0),
    change_id BIGINT NOT NULL CHECK (change_id >= 1),
    coverage_revision BIGINT NOT NULL CHECK (coverage_revision >= 0),
    entity_kind TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    operation_kind TEXT NOT NULL,
    envelope_version BIGINT NOT NULL CHECK (envelope_version >= 1),
    fact_semantics_version BIGINT NOT NULL CHECK (fact_semantics_version >= 1),
    stats_semantics_version BIGINT NOT NULL CHECK (stats_semantics_version >= 1),
    occurred_at TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    payload JSONB NOT NULL,
    payload_bytes INTEGER NOT NULL CHECK (payload_bytes >= 0),
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (consumer_id, source_instance_id, event_id),
    UNIQUE (consumer_id, source_instance_id, generation, change_id)
);

CREATE INDEX IF NOT EXISTS analytics_module_ingest_receipts_cursor_idx
    ON cvm_analytics.analytics_module_ingest_receipts
        (consumer_id, source_instance_id, generation, change_id);

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_module_ingest_work (
    consumer_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    generation BIGINT NOT NULL CHECK (generation >= 0),
    change_id BIGINT NOT NULL CHECK (change_id >= 1),
    event_id TEXT NOT NULL,
    work_kind TEXT NOT NULL CHECK (work_kind IN ('projection', 'archive')),
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'completed', 'blocked')),
    attempts BIGINT NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error_code TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (consumer_id, source_instance_id, generation, change_id, work_kind),
    UNIQUE (consumer_id, source_instance_id, event_id, work_kind)
);

CREATE INDEX IF NOT EXISTS analytics_module_ingest_work_due_idx
    ON cvm_analytics.analytics_module_ingest_work (consumer_id, state, updated_at);

REVOKE ALL ON TABLE cvm_analytics.analytics_module_ingest_receipts
    FROM PUBLIC, cvm_analytics_app;
GRANT SELECT, INSERT ON TABLE cvm_analytics.analytics_module_ingest_receipts
    TO cvm_analytics_maintenance;
REVOKE ALL ON TABLE cvm_analytics.analytics_module_ingest_work
    FROM PUBLIC, cvm_analytics_app;
GRANT SELECT, INSERT, UPDATE ON TABLE cvm_analytics.analytics_module_ingest_work
    TO cvm_analytics_maintenance;
