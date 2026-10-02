use metrics::Label;

pub(super) fn mapped_metric(
    id: &'static str,
    dimension: &'static str,
) -> Option<(&'static str, Vec<Label>, f64)> {
    if !metric_dimensions(id).contains(&dimension) {
        return None;
    }
    let name = match id {
        "http.in_flight" => "cvm_http_inflight",
        "p1.ack_duration_ms" => "cvm_sqlite_batch_ack_duration_seconds",
        "p1.queue_depth" => "cvm_sqlite_pending_items",
        "p1.queue_bytes" => "cvm_sqlite_pending_bytes",
        "p2.queue_depth" => "cvm_sqlite_pending_items",
        "p1.retry_count" => "cvm_sqlite_retries_total",
        "p2.retry_count" => "cvm_sqlite_retries_total",
        "p1.transfer_bytes" => "cvm_sqlite_transfer_bytes_total",
        "p2.next_attempt_delay_ms" => "cvm_sqlite_next_attempt_delay_seconds",
        "p2.deferred_age_ms" => "cvm_sqlite_deferred_age_seconds",
        "p2.flush_attempt_count" => "cvm_sqlite_flush_attempts_total",
        "p2.pressure_defer_count" => "cvm_sqlite_defers_total",
        "p2.lock_retry_count" => "cvm_sqlite_retries_total",
        "sqlite.busy_count" => "cvm_sqlite_errors_total",
        "sqlite.locked_count" => "cvm_sqlite_errors_total",
        "sqlite.pool_timeout_count" => "cvm_sqlite_pool_timeouts_total",
        "sqlite.background_skip_count" => "cvm_sqlite_background_skips_total",
        "sqlite.write_duration_ms" => "cvm_sqlite_batch_execute_duration_seconds",
        "sqlite.write_rows" => "cvm_sqlite_written_rows_total",
        "sqlite.write_bytes" => "cvm_sqlite_written_bytes_total",
        "sqlite.wal_bytes" => "cvm_sqlite_wal_bytes",
        "sqlite.coordinator_waiters" => "cvm_sqlite_coordinator_waiters",
        "sqlite.coordinator_wait_duration_ms" => "cvm_sqlite_coordinator_wait_duration_seconds",
        "sqlite.coordinator_bypass_count" => "cvm_sqlite_coordinator_bypasses_total",
        "sqlite.maintenance_fairness_count" => "cvm_sqlite_maintenance_fairness_total",
        "projection.last_good_age_ms" => "cvm_projection_last_good_age_seconds",
        "projection.build_count" => "cvm_projection_builds_total",
        "projection.live_db_read_count" => "cvm_projection_live_db_reads_total",
        "sse.active_subscribers" => "cvm_sse_active_subscribers",
        "projection.cadence_miss_count" => "cvm_projection_cadence_misses_total",
        "projection.revision_count" => "cvm_projection_revision_changes_total",
        "projection.publish_duration_ms" => "cvm_projection_publish_duration_seconds",
        "projection.publish_count" => "cvm_projection_publications_total",
        "projection.snapshot_bytes" => "cvm_projection_snapshot_bytes",
        "projection.reconcile_duration_ms" => "cvm_projection_reconcile_duration_seconds",
        "projection.reconcile_count" => "cvm_projection_reconciliations_total",
        "projection.reconcile_defer_count" => "cvm_projection_reconciliations_total",
        "projection.reconcile_failure_count" => "cvm_projection_reconciliations_total",
        "sse.publish_error_count" => "cvm_sse_publish_errors_total",
        "sse.frame_bytes" => "cvm_sse_frame_bytes_total",
        "maintenance.backlog_age_ms" => "cvm_maintenance_backlog_age_seconds",
        "maintenance.run_duration_ms" => "cvm_maintenance_run_duration_seconds",
        "maintenance.processed_rows" => "cvm_maintenance_processed_rows_total",
        "maintenance.raw_bytes_before" => "cvm_maintenance_raw_bytes",
        "maintenance.raw_bytes_after" => "cvm_maintenance_raw_bytes",
        "maintenance.compressed_file_count" => "cvm_maintenance_files_total",
        "maintenance.removed_file_count" => "cvm_maintenance_files_total",
        "maintenance.archived_rows" => "cvm_maintenance_archived_rows_total",
        "process.rss_bytes" => "cvm_process_rss_bytes",
        "process.cpu_percent" => "cvm_process_cpu_seconds_total",
        "process.rss_anon_bytes" => "cvm_process_rss_anon_bytes",
        "process.swap_bytes" => "cvm_process_swap_bytes",
        "process.managed_bytes" => "cvm_process_managed_bytes",
        "process.unattributed_anon_bytes" => "cvm_process_unattributed_anon_bytes",
        "storage.main_db_bytes" => "cvm_storage_database_bytes",
        "process.thread_count" => "cvm_process_threads",
        "process.disk_free_bytes" => "cvm_storage_available_bytes",
        "browser.data_ready_ms" => "cvm_browser_data_ready_seconds",
        "browser.update_to_paint_ms" => "cvm_browser_update_to_paint_seconds",
        "browser.long_task_ms" => "cvm_browser_long_task_duration_seconds",
        "browser.long_task_count" => "cvm_browser_events_total",
        "browser.api_request_duration_ms" => "cvm_browser_api_duration_seconds",
        "browser.api_request_count" => "cvm_browser_events_total",
        "browser.sse_duration_ms" => "cvm_browser_sse_duration_seconds",
        "browser.sse_disconnect_count" => "cvm_browser_sse_ends_total",
        "browser.unsupported_count" => "cvm_browser_unsupported_total",
        "browser.visibility_hidden_count" => "cvm_browser_visibility_hidden_total",
        "maintenance.task_run_duration_ms" => "cvm_task_run_duration_seconds",
        _ => return None,
    };
    let mut labels = Vec::new();
    match id {
        "p1.queue_depth" | "p1.queue_bytes" => labels.push(Label::new("queue", "all")),
        "p2.queue_depth" => labels.push(Label::new("queue", "p2_derived")),
        "p1.transfer_bytes" => {
            labels.push(Label::new("from", "p1_terminal"));
            labels.push(Label::new("to", "p2_derived"));
        }
        "p1.retry_count" | "p2.retry_count" | "p2.lock_retry_count" => {
            labels.push(Label::new(
                "class",
                if id.starts_with("p1.") {
                    "p1_terminal"
                } else {
                    "p2_derived"
                },
            ));
            labels.push(Label::new(
                "reason",
                if id == "p2.lock_retry_count" {
                    "lock"
                } else {
                    "other"
                },
            ));
        }
        "p2.pressure_defer_count" => {
            labels.push(Label::new("class", "p2_derived"));
            labels.push(Label::new("reason", "pressure"));
        }
        id if id.starts_with("p1.") || id.starts_with("p2.") => labels.push(Label::new(
            "class",
            if id.starts_with("p1.") {
                "p1_terminal"
            } else {
                "p2_derived"
            },
        )),
        "sqlite.busy_count" | "sqlite.locked_count" => labels.push(Label::new(
            "kind",
            if id == "sqlite.busy_count" {
                "busy"
            } else {
                "locked"
            },
        )),
        "sqlite.wal_bytes" | "storage.main_db_bytes" => labels.push(Label::new("database", "main")),
        "sqlite.coordinator_waiters" | "sqlite.coordinator_wait_duration_ms" => {
            labels.push(Label::new("class", dimension))
        }
        "projection.reconcile_count"
        | "projection.reconcile_defer_count"
        | "projection.reconcile_failure_count" => {
            labels.push(Label::new(
                "outcome",
                if id.contains("defer") {
                    "deferred"
                } else if id.contains("failure") {
                    "error"
                } else {
                    "success"
                },
            ));
            labels.push(Label::new(
                "reason",
                if id.contains("defer") {
                    dimension
                } else {
                    "none"
                },
            ));
        }
        "projection.last_good_age_ms"
        | "projection.build_count"
        | "projection.live_db_read_count"
        | "projection.reconcile_duration_ms" => labels.push(Label::new("projection", "dashboard")),
        id if id.starts_with("projection.") || id.starts_with("sse.") => labels.push(Label::new(
            if id == "sse.active_subscribers" {
                "topic"
            } else {
                "slice"
            },
            dimension,
        )),
        "maintenance.task_run_duration_ms" => labels.push(Label::new("task_key", dimension)),
        "maintenance.raw_bytes_before" | "maintenance.raw_bytes_after" => labels.push(Label::new(
            "phase",
            if id.ends_with("before") {
                "before"
            } else {
                "after"
            },
        )),
        "maintenance.compressed_file_count" | "maintenance.removed_file_count" => {
            labels.push(Label::new(
                "action",
                if id.contains("compressed") {
                    "compressed"
                } else {
                    "removed"
                },
            ))
        }
        id if id.starts_with("maintenance.") => labels.push(Label::new("operation", "retention")),
        "process.disk_free_bytes" => labels.push(Label::new("filesystem", "data")),
        "browser.data_ready_ms" | "browser.update_to_paint_ms" => {
            let (page, device) = dimension.split_once(':')?;
            labels.push(Label::new("page", page));
            labels.push(Label::new("device", device));
        }
        id if id.starts_with("browser.") => {
            labels.push(Label::new("page", dimension));
            if id == "browser.long_task_count" || id == "browser.api_request_count" {
                labels.push(Label::new(
                    "event",
                    if id.contains("long_task") {
                        "long_task"
                    } else {
                        "api_request"
                    },
                ));
            }
        }
        _ => {}
    }
    let scale = if id.ends_with("_ms") { 0.001 } else { 1.0 };
    Some((name, labels, scale))
}

pub(super) fn metric_dimensions(metric_id: &str) -> &'static [&'static str] {
    match metric_id {
        "http.in_flight" => &["other"],
        id if id.starts_with("p1.") => &["p1"],
        id if id.starts_with("p2.") => &["p2"],
        "sqlite.coordinator_waiters" | "sqlite.coordinator_wait_duration_ms" => &[
            "p1_terminal",
            "interactive_proxy",
            "p2_derived",
            "maintenance_retention",
        ],
        "sqlite.coordinator_bypass_count" | "sqlite.maintenance_fairness_count" => &["coordinator"],
        id if id.starts_with("sqlite.") => &["main"],
        "projection.last_good_age_ms"
        | "projection.build_count"
        | "projection.live_db_read_count" => &["dashboard"],
        "projection.cadence_miss_count"
        | "projection.revision_count"
        | "projection.publish_duration_ms"
        | "projection.publish_count"
        | "projection.snapshot_bytes" => &["current", "network", "terminal"],
        "projection.reconcile_duration_ms" | "projection.reconcile_count" => &["dashboard"],
        "projection.reconcile_defer_count" => &["writer_pressure", "background_busy"],
        "projection.reconcile_failure_count" => &["dashboard"],
        "sse.active_subscribers" => &["dashboard"],
        "sse.publish_error_count" | "sse.publish_duration_ms" | "sse.frame_bytes" => {
            &["current", "network", "terminal"]
        }
        "maintenance.task_run_duration_ms" => TASK_RUN_METRIC_DIMENSIONS,
        "maintenance.task_run_count"
        | "maintenance.task_run_success_count"
        | "maintenance.task_run_failure_count" => TASK_RUN_METRIC_DIMENSIONS,
        id if id.starts_with("maintenance.") => &["maintenance"],
        id if id.starts_with("process.") => &["process"],
        "storage.main_db_bytes" => &["main_db"],
        "storage.telemetry_db_bytes" => &["telemetry_db"],
        "storage.telemetry_wal_bytes" => &["telemetry_wal"],
        id if id.starts_with("telemetry.") => &["collector"],
        "browser.data_ready_ms" | "browser.update_to_paint_ms" => &[
            "dashboard:mobile",
            "dashboard:desktop",
            "records:mobile",
            "records:desktop",
            "system:mobile",
            "system:desktop",
        ],
        id if id.starts_with("browser.") => &["dashboard", "records", "system"],
        _ => &[],
    }
}

const TASK_RUN_METRIC_DIMENSIONS: &[&str] = &[
    "retention_archive",
    "upstream_account_maintenance",
    "forward_proxy_subscription_refresh",
    "pool_orphan_recovery",
    "startup_hourly_rollup_bootstrap",
    "system_status_snapshot",
    "invocation_timeline_snapshot",
    "summary_snapshot",
    "summary_coverage_recovery",
    "dashboard_runtime_projection_reconcile",
    "long_term_projection",
    "timeseries_minute_projection",
    "raw_payload_metrics_inventory",
    "prompt_cache_materialization",
    "startup_backfill",
    "raw_compression",
    "archive_upstream_activity_manifest",
    "materialize_historical_rollups",
    "verify_archive_storage",
    "prune_archive_batches",
    "prune_legacy_archive_batches",
    "startup_backfill_child",
    "unknown_managed_task",
];
