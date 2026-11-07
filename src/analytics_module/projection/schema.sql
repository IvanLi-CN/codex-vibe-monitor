-- Versioned facts are immutable. A tombstone is a version, never a physical delete.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_facts (
    consumer_id TEXT NOT NULL,
    build_id TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    change_id BIGINT NOT NULL CHECK (change_id >= 0),
    deleted BOOLEAN NOT NULL,
    payload JSONB NOT NULL,
    PRIMARY KEY (consumer_id, build_id, entity_kind, entity_key, change_id)
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_progress (
    consumer_id TEXT NOT NULL,
    build_id TEXT NOT NULL,
    materialized_through BIGINT NOT NULL CHECK (materialized_through >= 0),
    PRIMARY KEY (consumer_id, build_id)
);
-- Each staging snapshot belongs to one immutable publication token.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_models (
    publication_id TEXT NOT NULL,
    model_kind TEXT NOT NULL,
    model_key TEXT NOT NULL,
    payload JSONB NOT NULL,
    PRIMARY KEY (publication_id, model_kind, model_key)
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_pins (
    pin_id TEXT PRIMARY KEY,
    consumer_id TEXT NOT NULL,
    build_id TEXT NOT NULL,
    publication_id TEXT,
    through_change_id BIGINT NOT NULL CHECK (through_change_id >= 0),
    pin_kind TEXT NOT NULL CHECK (pin_kind IN ('reader', 'build')),
    expires_at TIMESTAMPTZ NOT NULL
);
REVOKE ALL ON TABLE cvm_analytics.analytics_projection_facts,
    cvm_analytics.analytics_projection_progress, cvm_analytics.analytics_projection_models,
    cvm_analytics.analytics_projection_pins FROM PUBLIC, cvm_analytics_app;
GRANT SELECT, INSERT ON TABLE cvm_analytics.analytics_projection_facts,
    cvm_analytics.analytics_projection_models TO cvm_analytics_maintenance;
GRANT SELECT, INSERT, UPDATE ON TABLE cvm_analytics.analytics_projection_progress
    TO cvm_analytics_maintenance;
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE cvm_analytics.analytics_projection_pins
    TO cvm_analytics_maintenance;
GRANT SELECT ON TABLE cvm_analytics.analytics_projection_facts,
    cvm_analytics.analytics_projection_models TO cvm_analytics_reader;
-- Pins are control metadata. Readers can retain only a publication already admitted
-- by the publication/owner fence; query admission remains mandatory.
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE cvm_analytics.analytics_projection_pins
    TO cvm_analytics_reader;
