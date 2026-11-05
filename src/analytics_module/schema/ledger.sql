CREATE TABLE IF NOT EXISTS cvm_analytics.analytics_module_ddl_ledger (
    component TEXT NOT NULL,
    schema_version BIGINT NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    step_order BIGINT NOT NULL CHECK (step_order > 0),
    step_id TEXT NOT NULL,
    checksum TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('applying', 'applied', 'failed')),
    min_reader_version BIGINT NOT NULL CHECK (min_reader_version > 0),
    requires_bootstrap BOOLEAN NOT NULL DEFAULT FALSE,
    error_code TEXT CHECK (error_code IS NULL OR octet_length(error_code) <= 128),
    applied_at TIMESTAMPTZ,
    PRIMARY KEY (component, step_order),
    UNIQUE (component, step_id)
);

ALTER TABLE cvm_analytics.analytics_module_ddl_ledger
    ADD COLUMN IF NOT EXISTS schema_version BIGINT NOT NULL DEFAULT 1;
