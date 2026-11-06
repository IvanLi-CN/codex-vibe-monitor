-- Apply this contract through an owner-authorized migration connection after the
-- module DDL transaction. Role creation, credentials, and role membership remain
-- owner-managed provisioning operations; this file only reconciles object grants.
REVOKE ALL ON TABLE
    cvm_analytics.analytics_module_consumers,
    cvm_analytics.analytics_module_owners,
    cvm_analytics.analytics_module_builds,
    cvm_analytics.analytics_module_publications,
    cvm_analytics.analytics_module_repairs
    FROM PUBLIC;

REVOKE ALL ON TABLE
    cvm_analytics.analytics_module_consumers,
    cvm_analytics.analytics_module_owners,
    cvm_analytics.analytics_module_builds,
    cvm_analytics.analytics_module_publications,
    cvm_analytics.analytics_module_repairs
    FROM cvm_analytics_app, cvm_analytics_maintenance;

GRANT USAGE ON SCHEMA cvm_analytics TO cvm_analytics_reader;
GRANT SELECT (
    consumer_id, module_id, consumer_version, state, source_instance_id,
    owner_epoch, baseline_change_id, checkpoint, coverage_revision,
    active_build_id, active_publication_id, legacy_reader_fenced, updated_at
) ON TABLE cvm_analytics.analytics_module_consumers TO cvm_analytics_reader;
GRANT SELECT (
    module_id, owner_epoch, status, build_id, last_heartbeat, updated_at
) ON TABLE cvm_analytics.analytics_module_owners TO cvm_analytics_reader;
GRANT SELECT (
    build_id, module_id, consumer_id, source_instance_id, source_generation,
    source_high_watermark, source_coverage_revision, owner_epoch, state,
    projection_version, error_code, updated_at
) ON TABLE cvm_analytics.analytics_module_builds TO cvm_analytics_reader;
GRANT SELECT (
    publication_id, module_id, consumer_id, build_id, owner_epoch,
    published_through_change_id, coverage_revision, projection_version,
    reader_version, state, legacy_reader_fenced, published_at, updated_at
) ON TABLE cvm_analytics.analytics_module_publications TO cvm_analytics_reader;
GRANT SELECT (
    repair_id, module_id, consumer_id, build_id, owner_epoch,
    from_change_id, through_change_id, reason_code, state, attempts,
    last_error_code, updated_at
) ON TABLE cvm_analytics.analytics_module_repairs TO cvm_analytics_reader;
GRANT USAGE ON SCHEMA cvm_analytics TO cvm_analytics_maintenance;
GRANT SELECT, UPDATE ON TABLE
    cvm_analytics.analytics_module_consumers,
    cvm_analytics.analytics_module_owners
    TO cvm_analytics_maintenance;
GRANT SELECT, INSERT, UPDATE ON TABLE
    cvm_analytics.analytics_module_builds,
    cvm_analytics.analytics_module_publications,
    cvm_analytics.analytics_module_repairs
    TO cvm_analytics_maintenance;

-- No application or maintenance role receives schema DDL, ledger mutation, or sequence access.
