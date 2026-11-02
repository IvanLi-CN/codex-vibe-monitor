#[path = "startup_rebuild_and_retention_basics/failure_classification_backfill.rs"]
mod failure_classification_backfill;
#[path = "startup_rebuild_and_retention_basics/hourly_rollup_bootstrap_and_startup_backfill.rs"]
mod hourly_rollup_bootstrap_and_startup_backfill;
#[path = "startup_rebuild_and_retention_basics/retention_and_archive_activity_recovery.rs"]
mod retention_and_archive_activity_recovery;
#[path = "startup_rebuild_and_retention_basics/schema_and_timeseries.rs"]
mod schema_and_timeseries;
#[path = "startup_rebuild_and_retention_basics/startup_health_and_hydration.rs"]
mod startup_health_and_hydration;

pub(super) use schema_and_timeseries::{
    assert_startup_backfill_busy_error_closes_gate_before_next_task,
    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task,
    seed_due_startup_backfill_progress,
};
