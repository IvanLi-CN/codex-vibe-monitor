-- A source pin receipt is registered independently of the proposed certificate.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_sources (
    consumer_id TEXT NOT NULL,
    build_id TEXT NOT NULL,
    snapshot JSONB NOT NULL,
    expected JSONB NOT NULL,
    trust JSONB NOT NULL,
    facts JSONB NOT NULL,
    seed_artifacts JSONB NOT NULL,
    PRIMARY KEY (consumer_id, build_id)
);
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_projection_baselines (
    consumer_id TEXT NOT NULL,
    build_id TEXT NOT NULL,
    through_change_id BIGINT NOT NULL CHECK (through_change_id >= 0),
    certificate_digest TEXT NOT NULL,
    prefix_digest TEXT NOT NULL,
    coverage JSONB NOT NULL,
    seeds JSONB NOT NULL,
    PRIMARY KEY (consumer_id, build_id)
);
REVOKE ALL ON TABLE cvm_analytics.analytics_projection_sources,
    cvm_analytics.analytics_projection_baselines FROM PUBLIC, cvm_analytics_app;
GRANT SELECT, INSERT ON TABLE cvm_analytics.analytics_projection_sources,
    cvm_analytics.analytics_projection_baselines TO cvm_analytics_maintenance;
GRANT SELECT ON TABLE cvm_analytics.analytics_projection_baselines TO cvm_analytics_reader;
