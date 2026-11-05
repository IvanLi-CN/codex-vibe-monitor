-- Analytics module schema unit v1. It is additive and safe to replay.
CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_module_consumers (
    consumer_id TEXT PRIMARY KEY,
    module_id TEXT NOT NULL,
    consumer_version BIGINT NOT NULL CHECK (consumer_version > 0),
    state TEXT NOT NULL CHECK (state IN ('building', 'active', 'retired')),
    source_instance_id TEXT NOT NULL,
    owner_epoch BIGINT NOT NULL CHECK (owner_epoch >= 0),
    baseline_change_id BIGINT NOT NULL DEFAULT 0 CHECK (baseline_change_id >= 0),
    checkpoint BIGINT NOT NULL DEFAULT 0 CHECK (checkpoint >= 0),
    coverage_revision BIGINT NOT NULL DEFAULT 0 CHECK (coverage_revision >= 0),
    retention_pin_id TEXT,
    active_build_id TEXT,
    active_publication_id TEXT,
    legacy_reader_fenced BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (module_id, consumer_version)
);

CREATE INDEX IF NOT EXISTS analytics_module_consumers_state_idx
    ON cvm_analytics.analytics_module_consumers (state, module_id, consumer_id);

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_module_owners (
    module_id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    owner_epoch BIGINT NOT NULL CHECK (owner_epoch >= 0),
    status TEXT NOT NULL CHECK (status IN ('fenced', 'claiming', 'active', 'released')),
    build_id TEXT,
    last_heartbeat TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_module_builds (
    build_id TEXT PRIMARY KEY,
    module_id TEXT NOT NULL,
    consumer_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    source_generation BIGINT NOT NULL CHECK (source_generation >= 0),
    source_high_watermark BIGINT NOT NULL CHECK (source_high_watermark >= 0),
    source_coverage_revision BIGINT NOT NULL CHECK (source_coverage_revision >= 0),
    owner_epoch BIGINT NOT NULL CHECK (owner_epoch >= 0),
    state TEXT NOT NULL CHECK (state IN ('pending', 'building', 'ready', 'failed', 'retired')),
    retention_pin_id TEXT,
    projection_version BIGINT NOT NULL CHECK (projection_version > 0),
    error_code TEXT CHECK (error_code IS NULL OR octet_length(error_code) <= 128),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS analytics_module_builds_consumer_idx
    ON cvm_analytics.analytics_module_builds (consumer_id, state, updated_at);

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_module_publications (
    publication_id TEXT PRIMARY KEY,
    module_id TEXT NOT NULL,
    consumer_id TEXT NOT NULL,
    build_id TEXT NOT NULL,
    owner_epoch BIGINT NOT NULL CHECK (owner_epoch >= 0),
    published_through_change_id BIGINT NOT NULL CHECK (published_through_change_id >= 0),
    coverage_revision BIGINT NOT NULL CHECK (coverage_revision >= 0),
    projection_version BIGINT NOT NULL CHECK (projection_version > 0),
    reader_version BIGINT NOT NULL CHECK (reader_version > 0),
    state TEXT NOT NULL CHECK (state IN ('staging', 'published', 'superseded', 'invalidated')),
    legacy_reader_fenced BOOLEAN NOT NULL DEFAULT TRUE,
    published_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (consumer_id, build_id, projection_version)
);

CREATE INDEX IF NOT EXISTS analytics_module_publications_reader_idx
    ON cvm_analytics.analytics_module_publications
        (consumer_id, state, owner_epoch, published_through_change_id);

CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_module_repairs (
    repair_id TEXT PRIMARY KEY,
    module_id TEXT NOT NULL,
    consumer_id TEXT NOT NULL,
    build_id TEXT NOT NULL,
    owner_epoch BIGINT NOT NULL CHECK (owner_epoch >= 0),
    from_change_id BIGINT NOT NULL CHECK (from_change_id >= 0),
    through_change_id BIGINT NOT NULL CHECK (through_change_id >= from_change_id),
    reason_code TEXT NOT NULL CHECK (octet_length(reason_code) <= 128),
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'resolved', 'blocked')),
    attempts BIGINT NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error_code TEXT CHECK (last_error_code IS NULL OR octet_length(last_error_code) <= 128),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS analytics_module_repairs_due_idx
    ON cvm_analytics.analytics_module_repairs (consumer_id, state, updated_at);
