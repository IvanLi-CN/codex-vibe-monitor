use super::*;

#[test]
fn failure_kind_prefix_is_removed_only_when_it_matches_exactly() {
    let input = "[upstream_response_failed] upstream stream error";

    assert_eq!(
        strip_matching_failure_kind_prefix(input, Some("upstream_response_failed")),
        "upstream stream error",
    );
    assert_eq!(
        strip_matching_failure_kind_prefix(input, Some("different_failure_kind")),
        input,
    );
    assert_eq!(strip_matching_failure_kind_prefix(input, None), input);
    assert_eq!(
        strip_matching_failure_kind_prefix(
            "upstream stream error",
            Some("upstream_response_failed")
        ),
        "upstream stream error",
    );
}

#[test]
fn failure_kind_prefix_requires_a_boundary_after_the_closing_bracket() {
    assert_eq!(
        strip_matching_failure_kind_prefix(
            "[upstream_response_failed]",
            Some("upstream_response_failed"),
        ),
        "",
    );
    assert_eq!(
        strip_matching_failure_kind_prefix(
            "[upstream_response_failed]upstream stream error",
            Some("upstream_response_failed"),
        ),
        "[upstream_response_failed]upstream stream error",
    );
    assert_eq!(
        strip_matching_failure_kind_prefix(
            "[upstream_response_failed] [provider] upstream stream error",
            Some("upstream_response_failed"),
        ),
        "[provider] upstream stream error",
    );
}

#[test]
fn error_reason_categorization_preserves_unmatched_bracketed_text() {
    assert_eq!(
        categorize_error_with_failure_kind("[provider] upstream stream error", None),
        "[provider] upstream stream error",
    );
    assert_eq!(
        categorize_error_with_failure_kind(
            "[provider] upstream stream error",
            Some("different_failure_kind"),
        ),
        "[provider] upstream stream error",
    );
    assert_eq!(
        categorize_error_with_failure_kind(
            "[upstream_response_failed] upstream stream error",
            Some("upstream_response_failed"),
        ),
        "upstream stream error",
    );
}

fn live_record(
    invoke_id: &str,
    account_id: Option<i64>,
    status: &str,
    phase: Option<&str>,
    attempts: i64,
) -> ApiInvocation {
    ApiInvocation {
        id: 1,
        invoke_id: invoke_id.to_string(),
        occurred_at: "2026-07-12 10:00:00".to_string(),
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: None,
        model: None,
        request_model: None,
        response_model: None,
        input_tokens: None,
        output_tokens: None,
        cache_input_tokens: None,
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        reasoning_effort: None,
        total_tokens: None,
        cost: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        cache_write_tokens: None,
        status: Some(status.to_string()),
        live_phase: phase.map(str::to_string),
        error_message: None,
        downstream_status_code: None,
        failure_kind: None,
        blocked_binding: None,
        blocked_binding_json: None,
        stream_terminal_event: None,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        failure_class: None,
        is_actionable: None,
        endpoint: None,
        compaction_request_kind: None,
        compaction_response_kind: None,
        image_intent: None,
        requester_ip: None,
        prompt_cache_key: None,
        sticky_key: None,
        route_mode: None,
        upstream_account_id: account_id,
        upstream_account_name: None,
        response_content_encoding: None,
        request_compression_algorithm: None,
        transport: None,
        pool_attempt_count: Some(attempts),
        pool_distinct_account_count: None,
        pool_attempt_terminal_reason: None,
        requested_service_tier: None,
        service_tier: None,
        billing_service_tier: None,
        proxy_weight_delta: None,
        cost_estimated: None,
        price_version: None,
        cost_audit: None,
        request_raw_path: None,
        request_raw_size: None,
        request_raw_truncated: None,
        request_raw_truncated_reason: None,
        response_raw_path: None,
        response_raw_size: None,
        response_raw_truncated: None,
        response_raw_truncated_reason: None,
        detail_level: "full".to_string(),
        detail_pruned_at: None,
        detail_prune_reason: None,
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
        created_at: "2026-07-12 10:00:00".to_string(),
    }
}

fn responding_live_record(
    invoke_id: &str,
    account_id: Option<i64>,
    attempts: i64,
) -> ApiInvocation {
    let mut record = live_record(
        invoke_id,
        account_id,
        "running",
        Some("responding"),
        attempts,
    );
    record.first_token_ms = Some(700.0);
    record
}

#[test]
fn api_invocation_timing_serialization_drops_invalid_values() {
    let mut record = live_record("timing-safety", None, "success", None, 1);
    record.t_total_ms = Some(-1.0);
    record.first_token_ms = Some(0.0);
    record.t_upstream_stream_ms = Some(0.0);
    record.t_resp_parse_ms = Some(f64::INFINITY);

    let payload = serde_json::to_value(&record).expect("serialize invocation timing");

    assert!(payload["tTotalMs"].is_null());
    assert_eq!(payload["firstTokenMs"], 0.0);
    assert!(payload["tUpstreamStreamMs"].is_null());
    assert!(payload["tRespParseMs"].is_null());
}

#[test]
fn dashboard_activity_live_snapshot_groups_one_runtime_read_by_account() {
    let snapshot = build_dashboard_activity_live_snapshot(
        9,
        [
            live_record("c-1", Some(42), "running", Some("requesting"), 1),
            responding_live_record("c-2", Some(42), 2),
            live_record("u-1", None, "running", None, 1),
            live_record("done", Some(42), "success", None, 1),
        ],
    );

    assert_eq!(snapshot.revision, 9);
    assert_eq!(snapshot.in_progress_invocation_count, 3);
    assert_eq!(snapshot.retry_invocation_count, 1);
    assert_eq!(snapshot.in_progress_phase_counts.queued, 1);
    assert_eq!(snapshot.in_progress_phase_counts.requesting, 2);
    assert_eq!(snapshot.in_progress_phase_counts.responding, 0);
    assert_eq!(snapshot.accounts.len(), 2);
    let account = snapshot
        .accounts
        .iter()
        .find(|row| row.upstream_account_id == Some(42))
        .unwrap();
    assert_eq!(account.in_progress_invocation_count, 2);
    assert_eq!(account.retry_invocation_count, 1);
}

#[test]
fn dashboard_activity_live_snapshot_infers_missing_runtime_phase() {
    let mut requesting = live_record("requesting", Some(42), "running", None, 1);
    requesting.t_upstream_connect_ms = Some(4.0);
    let mut responding = live_record("responding", Some(42), "running", None, 1);
    responding.t_upstream_ttfb_ms = Some(12.0);
    responding.first_token_ms = Some(12.0);

    let snapshot = build_dashboard_activity_live_snapshot(10, [requesting, responding]);

    assert_eq!(snapshot.in_progress_phase_counts.queued, 0);
    assert_eq!(snapshot.in_progress_phase_counts.requesting, 1);
    assert_eq!(snapshot.in_progress_phase_counts.responding, 1);
    assert_eq!(snapshot.in_progress_wait_sample_count, 1);
    assert_eq!(snapshot.in_progress_wait_sum_ms, 12.0);
}

#[test]
fn dashboard_activity_live_snapshot_rejects_stale_responding_without_first_token() {
    let record = live_record(
        "stale-responding",
        Some(42),
        "running",
        Some("responding"),
        1,
    );

    let snapshot = build_dashboard_activity_live_snapshot(11, [record]);

    assert_eq!(snapshot.in_progress_phase_counts.requesting, 1);
    assert_eq!(snapshot.in_progress_phase_counts.responding, 0);
}

#[test]
fn dashboard_network_cache_removes_stale_retry_ttft_sample() {
    let observed_at = Utc::now();
    let cache = DashboardNetworkSpeedCache::new(observed_at);
    let first_attempt = responding_live_record("retry-network", Some(42), 1);
    cache.observe_dashboard_activity_runtime_snapshot(&first_attempt, observed_at);

    assert_eq!(
        cache
            .snapshot_dashboard_activity_accounts(observed_at)
            .get(&Some(42))
            .map(|account| account.first_token_sample_count),
        Some(1)
    );

    let mut retry = first_attempt.clone();
    retry.pool_attempt_count = Some(2);
    cache.observe_dashboard_activity_runtime_snapshot(&retry, observed_at);
    assert!(
        cache
            .snapshot_dashboard_activity_accounts(observed_at)
            .get(&Some(42))
            .is_none_or(|account| account.first_token_sample_count == 0)
    );

    cache.finalize_dashboard_activity_invocation(&retry, observed_at);
    assert!(
        cache
            .snapshot_dashboard_activity_accounts(observed_at)
            .get(&Some(42))
            .is_none_or(|account| account.first_token_sample_count == 0)
    );
}

#[test]
fn dashboard_activity_live_revision_reservation_is_monotonic() {
    let first = reserve_dashboard_activity_live_revision();
    let second = reserve_dashboard_activity_live_revision();

    assert_eq!(second, first + 1);
}

#[test]
fn runtime_projection_mode_rejects_removed_legacy_kill_switch() {
    assert_eq!(
        RuntimeProjectionMode::parse(None).expect("default projection mode"),
        RuntimeProjectionMode::Auto
    );
    assert!(RuntimeProjectionMode::parse(Some("legacy")).is_err());
    assert!(RuntimeProjectionMode::parse(Some("invalid")).is_err());
}

#[test]
fn runtime_projection_fixed_deadline_is_not_extended_by_later_mutations() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    let started_at = Instant::now();

    hub.mark_dashboard_dirty_at("runtime_upsert", started_at);
    let first_deadline = hub
        .pending_dashboard_deadline()
        .expect("first mutation should establish a deadline");
    hub.mark_dashboard_dirty_at("network_delta", started_at + Duration::from_millis(200));

    assert_eq!(
        first_deadline.duration_since(started_at),
        Duration::from_millis(250)
    );
    assert_eq!(hub.pending_dashboard_deadline(), Some(first_deadline));
}

#[test]
fn runtime_projection_build_does_not_extend_next_fixed_deadline() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    let started_at = Instant::now();
    hub.mark_dashboard_dirty_at("runtime_upsert", started_at);
    let pending = hub
        .pending_dashboard_publish_window()
        .expect("first mutation should establish a publish window");
    let building = hub
        .begin_dashboard_publish_window(pending)
        .expect("pending window should begin building");
    let mutation_during_build = started_at + Duration::from_millis(300);

    hub.mark_dashboard_dirty_at("network_delta", mutation_during_build);
    let next_deadline = hub
        .pending_dashboard_deadline()
        .expect("mutation during build should establish the next deadline");
    hub.complete_dashboard_publish_window(building);

    assert_eq!(
        next_deadline,
        mutation_during_build + DASHBOARD_RUNTIME_PROJECTION_COALESCE
    );
    assert_eq!(hub.pending_dashboard_deadline(), Some(next_deadline));
}

#[test]
fn terminal_rollback_marks_runtime_projection_dirty() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    let record = live_record("terminal-rollback", Some(42), "success", None, 1);
    let invoke_id = record.invoke_id.clone();
    let occurred_at = record.occurred_at.clone();

    hub.upsert_terminal(record);
    let generation_before_rollback = hub.dashboard_generation();

    assert!(hub.clear_terminal_tombstone(&invoke_id, &occurred_at));
    assert_eq!(hub.dashboard_generation(), generation_before_rollback + 1);
}

#[test]
fn persisted_terminal_tombstone_insert_and_refresh_mark_projection_dirty() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    let generation_before_insert = hub.dashboard_generation();

    assert!(!hub.remove_persisted_terminal_overlay("persisted", "2026-08-04 12:00:00"));
    assert_eq!(hub.dashboard_generation(), generation_before_insert + 1);

    let generation_before_refresh = hub.dashboard_generation();
    assert!(!hub.remove_persisted_terminal_overlay("persisted", "2026-08-04 12:00:00"));
    assert_eq!(hub.dashboard_generation(), generation_before_refresh + 1);
}

#[test]
fn healthy_runtime_projection_renders_ten_thousand_mutations_without_sql() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    for mutation in 0..10_000 {
        let mut record = live_record(
            &format!("high-frequency-{mutation}"),
            Some(42),
            "running",
            Some("queued"),
            1,
        );
        record.live_phase = Some(if mutation % 2 == 0 {
            "requesting".to_string()
        } else {
            "responding".to_string()
        });
        hub.upsert(record.clone());
    }

    let mut render_samples_ms = Vec::new();
    let mut snapshot = None;
    for _ in 0..20 {
        let started_at = Instant::now();
        snapshot = Some(
            hub.dashboard_live_projection()
                .snapshot()
                .expect("healthy memory projection snapshot"),
        );
        render_samples_ms.push(started_at.elapsed().as_secs_f64() * 1_000.0);
    }
    render_samples_ms.sort_by(f64::total_cmp);
    let p95_index = ((render_samples_ms.len() - 1) as f64 * 0.95).ceil() as usize;
    let p95_ms = render_samples_ms[p95_index];
    let snapshot = snapshot.expect("projection snapshot");
    let health = hub.health_snapshot(0);

    assert_eq!(hub.runtime_record_count(), 10_000);
    assert_eq!(snapshot.in_progress_invocation_count, 10_000);
    assert_eq!(health.live_path_db_read_count, 0);
    assert!(
        p95_ms <= 400.0,
        "projection p95 exceeded 400ms: {p95_ms:.2}ms"
    );
}

#[test]
fn runtime_projection_preserves_restored_rows_across_later_mutations() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    let restored = live_record("restored-only", Some(41), "running", Some("requesting"), 1);
    let baseline_snapshot = build_dashboard_activity_live_snapshot(0, vec![restored.clone()]);
    let baseline = DashboardRuntimeProjectionBaseline {
        records: vec![DashboardRuntimeBaselineRecord {
            key: RuntimeInvocationKey::new(
                restored.invoke_id.clone(),
                restored.occurred_at.clone(),
            ),
            upstream_account_id: restored.upstream_account_id,
            upstream_account_name: restored.upstream_account_name.clone(),
            is_retry: false,
            live_phase: restored.live_phase.clone(),
            wait_ms: restored.t_upstream_ttfb_ms,
        }],
        source_scope: InvocationSourceScope::All,
        network_open_buckets: HashMap::new(),
    };
    let installed = hub
        .install_persistence_baseline_if_generation(
            baseline_snapshot,
            baseline,
            "startup_restore",
            hub.dashboard_generation(),
        )
        .expect("install startup baseline")
        .expect("baseline generation should match");
    assert_eq!(installed.snapshot.revision, 1);

    hub.upsert(responding_live_record("runtime-only", Some(42), 1));
    let capture = hub
        .capture_memory_snapshot()
        .expect("merged runtime projection snapshot");

    assert_eq!(capture.snapshot.revision, 2);
    assert_eq!(capture.snapshot.in_progress_invocation_count, 2);
    assert_eq!(capture.snapshot.in_progress_phase_counts.requesting, 1);
    assert_eq!(capture.snapshot.in_progress_phase_counts.responding, 1);
    assert_eq!(hub.health_snapshot(1).live_path_db_read_count, 0);

    let mut restored_update = restored.clone();
    restored_update.live_phase = Some("responding".to_string());
    restored_update.first_token_ms = Some(700.0);
    hub.upsert(restored_update.clone());
    let updated = hub
        .capture_memory_snapshot()
        .expect("runtime overlay should replace its restored row");
    assert_eq!(updated.snapshot.in_progress_invocation_count, 2);
    assert_eq!(updated.snapshot.in_progress_phase_counts.requesting, 0);
    assert_eq!(updated.snapshot.in_progress_phase_counts.responding, 2);

    restored_update.status = Some("completed".to_string());
    hub.upsert_terminal(restored_update);
    let terminal = hub
        .capture_memory_snapshot()
        .expect("terminal delta should remove its restored row");
    assert_eq!(terminal.snapshot.in_progress_invocation_count, 1);
    assert_eq!(terminal.snapshot.in_progress_phase_counts.responding, 1);
}

#[test]
fn runtime_projection_prune_removes_expired_records_from_live_snapshot() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    let expired = live_record(
        "expired-runtime",
        Some(41),
        "running",
        Some("requesting"),
        1,
    );
    let retained = responding_live_record("retained-runtime", Some(42), 1);
    hub.upsert(expired.clone());
    hub.upsert(retained);
    hub.backdate_for_test(
        &expired.invoke_id,
        &expired.occurred_at,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE + Duration::from_secs(1),
    );

    assert_eq!(hub.snapshot().len(), 1);
    let capture = hub
        .capture_memory_snapshot()
        .expect("pruned runtime projection snapshot");

    assert_eq!(capture.snapshot.in_progress_invocation_count, 1);
    assert_eq!(capture.snapshot.in_progress_phase_counts.requesting, 0);
    assert_eq!(capture.snapshot.in_progress_phase_counts.responding, 1);
}

#[test]
fn terminal_skipped_upsert_still_synchronizes_runtime_prune() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    let terminal = live_record("already-terminal", Some(41), "success", None, 1);
    let expired = live_record(
        "expired-before-skipped-upsert",
        Some(42),
        "running",
        Some("requesting"),
        1,
    );
    hub.upsert_terminal(terminal.clone());
    hub.upsert(expired.clone());
    hub.backdate_for_test(
        &expired.invoke_id,
        &expired.occurred_at,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE + Duration::from_secs(1),
    );

    let outcome = hub.upsert(terminal);
    let capture = hub
        .capture_memory_snapshot()
        .expect("skipped upsert prune projection snapshot");

    assert!(outcome.skipped_terminal);
    assert_eq!(outcome.pruned_count, 1);
    assert_eq!(capture.snapshot.in_progress_invocation_count, 0);
}

#[test]
fn runtime_remove_does_not_resurrect_baseline_record() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    let restored = live_record(
        "baseline-backed-runtime",
        Some(41),
        "running",
        Some("requesting"),
        1,
    );
    let baseline_snapshot = build_dashboard_activity_live_snapshot(0, vec![restored.clone()]);
    let baseline = DashboardRuntimeProjectionBaseline {
        records: vec![DashboardRuntimeBaselineRecord {
            key: RuntimeInvocationKey::new(
                restored.invoke_id.clone(),
                restored.occurred_at.clone(),
            ),
            upstream_account_id: restored.upstream_account_id,
            upstream_account_name: restored.upstream_account_name.clone(),
            is_retry: false,
            live_phase: restored.live_phase.clone(),
            wait_ms: restored.t_upstream_ttfb_ms,
        }],
        source_scope: InvocationSourceScope::All,
        network_open_buckets: HashMap::new(),
    };
    hub.install_persistence_baseline_if_generation(
        baseline_snapshot,
        baseline,
        "startup_restore",
        hub.dashboard_generation(),
    )
    .expect("install startup baseline")
    .expect("install baseline capture");
    hub.upsert(restored.clone());

    assert_eq!(
        hub.remove_non_terminal_by_invoke_id(&restored.invoke_id)
            .len(),
        1
    );
    let capture = hub
        .capture_memory_snapshot()
        .expect("removed baseline-backed projection snapshot");

    assert_eq!(capture.snapshot.in_progress_invocation_count, 0);
}

#[test]
fn runtime_prune_does_not_resurrect_baseline_record() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    let restored = live_record(
        "baseline-backed-prune",
        Some(41),
        "running",
        Some("requesting"),
        1,
    );
    let baseline_snapshot = build_dashboard_activity_live_snapshot(0, vec![restored.clone()]);
    let baseline = DashboardRuntimeProjectionBaseline {
        records: vec![DashboardRuntimeBaselineRecord {
            key: RuntimeInvocationKey::new(
                restored.invoke_id.clone(),
                restored.occurred_at.clone(),
            ),
            upstream_account_id: restored.upstream_account_id,
            upstream_account_name: restored.upstream_account_name.clone(),
            is_retry: false,
            live_phase: restored.live_phase.clone(),
            wait_ms: restored.t_upstream_ttfb_ms,
        }],
        source_scope: InvocationSourceScope::All,
        network_open_buckets: HashMap::new(),
    };
    hub.install_persistence_baseline_if_generation(
        baseline_snapshot,
        baseline,
        "startup_restore",
        hub.dashboard_generation(),
    )
    .expect("install startup baseline")
    .expect("install baseline capture");
    hub.upsert(restored.clone());
    hub.backdate_for_test(
        &restored.invoke_id,
        &restored.occurred_at,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE + Duration::from_secs(1),
    );

    assert!(hub.snapshot().is_empty());
    let capture = hub
        .capture_memory_snapshot()
        .expect("pruned baseline-backed projection snapshot");

    assert_eq!(capture.snapshot.in_progress_invocation_count, 0);
}

#[test]
fn runtime_remove_during_baseline_build_wins_over_stale_database_row() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    let stale = live_record(
        "removed-during-baseline",
        Some(41),
        "running",
        Some("requesting"),
        1,
    );
    hub.upsert(stale.clone());
    let expected_generation = hub.dashboard_generation();
    let stale_snapshot = build_dashboard_activity_live_snapshot(0, vec![stale.clone()]);
    let stale_baseline = DashboardRuntimeProjectionBaseline {
        records: vec![DashboardRuntimeBaselineRecord {
            key: RuntimeInvocationKey::new(stale.invoke_id.clone(), stale.occurred_at.clone()),
            upstream_account_id: stale.upstream_account_id,
            upstream_account_name: stale.upstream_account_name.clone(),
            is_retry: false,
            live_phase: stale.live_phase.clone(),
            wait_ms: stale.t_upstream_ttfb_ms,
        }],
        source_scope: InvocationSourceScope::All,
        network_open_buckets: HashMap::new(),
    };

    assert!(
        hub.remove_non_terminal(&stale.invoke_id, &stale.occurred_at)
            .is_some()
    );
    hub.install_persistence_baseline_if_generation(
        stale_snapshot,
        stale_baseline,
        "reconcile",
        expected_generation,
    )
    .expect("install baseline built before removal")
    .expect("install baseline capture");
    let capture = hub
        .capture_memory_snapshot()
        .expect("stale baseline row remains removed");

    assert_eq!(capture.snapshot.in_progress_invocation_count, 0);
}

#[test]
fn runtime_projection_accepts_baseline_built_before_runtime_mutation() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    let expected_generation = hub.dashboard_generation();
    let restored = live_record(
        "restored-before-race",
        Some(41),
        "running",
        Some("requesting"),
        1,
    );
    let baseline_snapshot = build_dashboard_activity_live_snapshot(0, vec![restored.clone()]);
    let baseline = DashboardRuntimeProjectionBaseline {
        records: vec![DashboardRuntimeBaselineRecord {
            key: RuntimeInvocationKey::new(
                restored.invoke_id.clone(),
                restored.occurred_at.clone(),
            ),
            upstream_account_id: restored.upstream_account_id,
            upstream_account_name: restored.upstream_account_name.clone(),
            is_retry: false,
            live_phase: restored.live_phase.clone(),
            wait_ms: restored.t_upstream_ttfb_ms,
        }],
        source_scope: InvocationSourceScope::All,
        network_open_buckets: HashMap::new(),
    };

    hub.upsert(responding_live_record(
        "runtime-during-baseline",
        Some(42),
        1,
    ));
    let installed = hub
        .install_persistence_baseline_if_generation(
            baseline_snapshot,
            baseline,
            "reconcile",
            expected_generation,
        )
        .expect("install raced baseline");

    assert!(installed.is_some());
    assert_eq!(
        installed.as_ref().map(|capture| capture.snapshot_origin),
        Some("reconcile_replayed")
    );
    let capture = hub
        .capture_memory_snapshot()
        .expect("baseline plus runtime mutation snapshot");
    assert_eq!(capture.snapshot.in_progress_invocation_count, 2);
    assert_eq!(capture.snapshot.in_progress_phase_counts.requesting, 1);
    assert_eq!(capture.snapshot.in_progress_phase_counts.responding, 1);
}

#[test]
fn runtime_projection_merges_restored_network_bucket_without_double_counting() {
    let now = Utc::now();
    let cache = Arc::new(DashboardNetworkSpeedCache::new(now));
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(cache.clone())
        .expect("bind dashboard network cache");
    let global_at_install = cache.snapshot_open_bucket(DashboardNetworkScopeKey::Global, now);
    let account_at_install = cache.snapshot_open_bucket(DashboardNetworkScopeKey::Account(42), now);
    let global_baseline_totals = DashboardNetworkByteTotals {
        upload_bytes: 120,
        download_bytes: 240,
    };
    let account_baseline_totals = DashboardNetworkByteTotals {
        upload_bytes: 80,
        download_bytes: 160,
    };
    let mut baseline_snapshot = build_dashboard_activity_live_snapshot(0, Vec::new());
    baseline_snapshot.network_live_bucket =
        Some(build_dashboard_network_timeseries_point_response(
            global_at_install.bucket_start,
            global_at_install.bucket_end,
            global_baseline_totals,
            ExactUtcRange {
                start: global_at_install.bucket_start,
                end: now,
            },
            true,
        ));
    let baseline = DashboardRuntimeProjectionBaseline {
        records: Vec::new(),
        source_scope: InvocationSourceScope::All,
        network_open_buckets: HashMap::from([
            (
                DashboardNetworkScopeKey::Global,
                DashboardRuntimeNetworkOpenBucketBaseline {
                    bucket_start: global_at_install.bucket_start,
                    bucket_end: global_at_install.bucket_end,
                    baseline_totals: global_baseline_totals,
                    memory_totals_at_install: global_at_install.totals,
                },
            ),
            (
                DashboardNetworkScopeKey::Account(42),
                DashboardRuntimeNetworkOpenBucketBaseline {
                    bucket_start: account_at_install.bucket_start,
                    bucket_end: account_at_install.bucket_end,
                    baseline_totals: account_baseline_totals,
                    memory_totals_at_install: account_at_install.totals,
                },
            ),
        ]),
    };
    hub.install_persistence_baseline_if_generation(
        baseline_snapshot,
        baseline,
        "startup_restore",
        hub.dashboard_generation(),
    )
    .expect("install startup baseline")
    .expect("baseline generation should match");

    cache.record_request_bytes(
        "network-delta",
        "2026-08-04 12:00:00",
        Some(42),
        Some("api.openai.com"),
        30,
        now,
    );
    cache.record_response_chunk_bytes(
        "network-delta",
        "2026-08-04 12:00:00",
        Some(42),
        Some("api.openai.com"),
        60,
        now,
    );
    hub.mark_dashboard_dirty_at("network_delta", Instant::now());

    let first = hub
        .capture_network_slice()
        .expect("merged network projection slice");
    let second = hub
        .capture_network_slice()
        .expect("unchanged merged network projection slice");

    let global = first
        .slice
        .network_live_bucket
        .as_ref()
        .expect("global live bucket");
    assert_eq!(global.upload_bytes, 150);
    assert_eq!(global.download_bytes, 300);
    let account = first
        .slice
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .and_then(|account| account.network_live_bucket.as_ref())
        .expect("account live bucket");
    assert_eq!(account.upload_bytes, 110);
    assert_eq!(account.download_bytes, 220);
    let second_global = second
        .slice
        .network_live_bucket
        .as_ref()
        .expect("second global live bucket");
    assert_eq!(second_global.upload_bytes, 150);
    assert_eq!(second_global.download_bytes, 300);
    let second_account = second
        .slice
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .and_then(|account| account.network_live_bucket.as_ref())
        .expect("second account live bucket");
    assert_eq!(second_account.upload_bytes, 110);
    assert_eq!(second_account.download_bytes, 220);
}

#[test]
fn network_projection_keeps_known_account_after_rate_bucket_expires() {
    let cache = DashboardNetworkSpeedCache::new(Utc::now());
    let known_account_ids = std::collections::BTreeSet::from([Some(42)]);

    let slice =
        DashboardNetworkProjectionSlice::from_memory(&cache, &HashMap::new(), &known_account_ids);

    let account = slice
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("known account remains in the network projection");
    assert_eq!(account.upload_bytes_per_second, 0.0);
    assert_eq!(account.download_bytes_per_second, 0.0);
    assert!(account.network_live_bucket.is_some());
}

#[tokio::test]
async fn dashboard_runtime_projection_update_p95_stays_within_four_hundred_ms() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let _lease = state
        .subscription_hub
        .register_test_topic_name("dashboard.activity.current")
        .await;
    let mut receiver = state.broadcaster.subscribe();
    let mut samples_ms = Vec::new();

    for mutation in 0..20 {
        let record = if mutation % 2 == 0 {
            live_record(
                "latency-contract",
                Some(42),
                "running",
                Some("requesting"),
                1,
            )
        } else {
            responding_live_record("latency-contract", Some(42), 1)
        };
        state.proxy_runtime_invocations.upsert(record);
        let started_at = Instant::now();
        schedule_dashboard_activity_live_snapshot(state.as_ref());
        let in_progress_invocation_count = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match receiver.recv().await {
                    Ok(BroadcastPayload::DashboardActivityLive { snapshot }) => {
                        return snapshot.in_progress_invocation_count;
                    }
                    Ok(BroadcastPayload::DashboardCurrentSlice { slice }) => {
                        return slice.in_progress_invocation_count;
                    }
                    _ => continue,
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "dashboard current update exceeded 1s at mutation {mutation}: {:?}",
                state.proxy_runtime_invocations.health_snapshot(1)
            )
        });
        assert_eq!(in_progress_invocation_count, 1);
        samples_ms.push(started_at.elapsed().as_secs_f64() * 1_000.0);
    }

    samples_ms.sort_by(f64::total_cmp);
    let p95_index = (samples_ms.len() * 95).div_ceil(100).saturating_sub(1);
    let p95_ms = samples_ms[p95_index];
    let health = state.proxy_runtime_invocations.health_snapshot(1);
    assert_eq!(health.live_path_db_read_count, 0);
    assert!(
        p95_ms <= 400.0,
        "dashboard current update p95 exceeded 400ms: {p95_ms:.2}ms"
    );
}

#[tokio::test]
async fn network_mutation_without_subscribers_is_dirty_for_first_snapshot() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    state.dashboard_network_speed_cache.record_request_bytes(
        "network-first-snapshot",
        "2026-08-04 12:00:00",
        Some(42),
        Some("api.openai.com"),
        128,
        Utc::now(),
    );

    schedule_dashboard_network_projection(state.as_ref());
    assert!(
        state
            .proxy_runtime_invocations
            .pending_dashboard_publish_window()
            .is_some_and(|window| window.slice == DashboardProjectionSlice::Network)
    );
    assert!(
        state
            .proxy_runtime_invocations
            .pending_dashboard_deadline()
            .is_none()
    );
    let snapshot = state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("first memory network slice after subscriber-free mutation");
    let health = state.proxy_runtime_invocations.health_snapshot(0);

    assert_eq!(
        snapshot
            .slice
            .network_live_bucket
            .expect("global live bucket")
            .upload_bytes,
        128
    );
    assert!(snapshot.changed);
    assert_eq!(health.live_path_db_read_count, 0);
}

#[tokio::test]
async fn network_only_schedule_does_not_construct_current_projection() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
        .expect("bind dashboard network cache");
    state
        .proxy_runtime_invocations
        .capture_memory_snapshot()
        .expect("establish current projection");
    state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("establish network projection");
    state
        .proxy_runtime_invocations
        .reset_dashboard_topology_counters();

    state.dashboard_network_speed_cache.record_request_bytes(
        "network-only-slice",
        "2026-08-04 12:00:00",
        Some(42),
        Some("api.openai.com"),
        256,
        Utc::now(),
    );
    schedule_dashboard_network_projection(state.as_ref());
    let capture = state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("capture network-only slice");
    let counters = state
        .proxy_runtime_invocations
        .dashboard_topology_counters();

    assert!(capture.changed);
    assert_eq!(counters.current.build_count, 0);
    assert_eq!(counters.current.revision_count, 0);
    assert_eq!(counters.network.build_count, 1);
    assert_eq!(counters.network.revision_count, 1);
    assert_eq!(
        state
            .proxy_runtime_invocations
            .health_snapshot(0)
            .live_path_db_read_count,
        0
    );
}

#[test]
fn active_network_slice_rearms_without_waking_current_projection() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.mark_dashboard_network_dirty();
    let pending = hub
        .pending_dashboard_publish_window()
        .expect("network publish window");
    let active = hub
        .begin_dashboard_publish_window(pending)
        .expect("begin network publish window");

    complete_dashboard_projection_publish_window(&hub, active, true);

    let rearmed = hub
        .pending_dashboard_publish_window()
        .expect("rearmed network publish window");
    assert_eq!(rearmed.slice, DashboardProjectionSlice::Network);
    assert!(hub.pending_dashboard_deadline().is_none());
}

#[test]
fn unchanged_runtime_projection_does_not_advance_revision() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    hub.upsert(live_record(
        "stable-revision",
        Some(42),
        "running",
        Some("requesting"),
        1,
    ));

    let first = hub
        .capture_memory_snapshot()
        .expect("first memory snapshot");
    let second = hub
        .capture_memory_snapshot()
        .expect("unchanged memory snapshot");

    assert!(first.changed);
    assert!(!second.changed);
    assert_eq!(second.snapshot.revision, first.snapshot.revision);
}

#[tokio::test]
async fn degraded_runtime_projection_reuses_last_good_without_database_read() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    state.proxy_runtime_invocations.upsert(live_record(
        "last-good",
        Some(42),
        "running",
        Some("requesting"),
        1,
    ));
    let first = capture_dashboard_activity_live_snapshot(state.as_ref())
        .await
        .expect("healthy memory snapshot");
    state
        .proxy_runtime_invocations
        .mark_degraded("test_health_gate");

    let degraded = capture_dashboard_activity_live_snapshot(state.as_ref())
        .await
        .expect("degraded last-good snapshot");
    let health = state.proxy_runtime_invocations.health_snapshot(1);

    assert_eq!(degraded.revision, first.revision);
    assert_eq!(health.live_path_db_read_count, 0);
    assert_eq!(health.snapshot_origin, "last_good");
    assert_eq!(health.state, "degraded");
}

#[test]
fn reconcile_failure_reports_degraded_health_without_freezing_memory_projection() {
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
    hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(Utc::now())))
        .expect("bind dashboard network cache");
    hub.upsert(live_record(
        "reconcile-health",
        Some(42),
        "running",
        Some("requesting"),
        1,
    ));
    hub.record_reconcile_failure("reconcile_failed");

    let snapshot = hub
        .capture_memory_snapshot()
        .expect("reconcile failure must not gate healthy memory projection");
    let health = hub.health_snapshot(1);

    assert_eq!(snapshot.snapshot.in_progress_invocation_count, 1);
    assert!(hub.is_memory_ready());
    assert_eq!(health.state, "degraded");
    assert_eq!(health.degraded_reason.as_deref(), Some("reconcile_failed"));
    assert_eq!(health.live_path_db_read_count, 0);
}

#[tokio::test]
async fn cold_runtime_projection_uses_exact_persistence_fallback_once() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;

    let snapshot = capture_dashboard_activity_live_snapshot(state.as_ref())
        .await
        .expect("cold persistence fallback");
    let health = state.proxy_runtime_invocations.health_snapshot(1);

    assert_eq!(snapshot.in_progress_invocation_count, 0);
    assert_eq!(health.live_path_db_read_count, 1);
    assert_eq!(health.snapshot_origin, "cold_fallback");
    assert_eq!(health.state, "healthy");
}

#[tokio::test]
async fn legacy_runtime_projection_kill_switch_keeps_persistence_path() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Legacy);
    hub.bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
        .expect("bind dashboard network cache");

    let first = capture_dashboard_activity_live_snapshot_from_runtime(
        &state.pool,
        &hub,
        state.dashboard_network_speed_cache.as_ref(),
    )
    .await
    .expect("first legacy capture");
    let second = capture_dashboard_activity_live_snapshot_from_runtime(
        &state.pool,
        &hub,
        state.dashboard_network_speed_cache.as_ref(),
    )
    .await
    .expect("second legacy capture");
    let health = hub.health_snapshot(0);

    assert!(first.changed);
    assert!(!second.changed);
    assert_eq!(second.snapshot.revision, first.snapshot.revision);
    assert_eq!(health.mode, "legacy");
    assert_eq!(health.live_path_db_read_count, 2);
}

#[test]
fn dashboard_activity_live_snapshot_serializes_network_realtime_rate() {
    let snapshot = DashboardActivityLiveSnapshot {
        revision: 11,
        generated_at: "2026-07-19T18:04:00.000Z".to_string(),
        in_progress_invocation_count: 0,
        in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
        retry_invocation_count: 0,
        in_progress_wait_sum_ms: 0.0,
        in_progress_wait_sample_count: 0,
        network_live_bucket: None,
        network_realtime_rate: Some(DashboardNetworkRealtimeRateResponse {
            sample_start: "2026-07-19T18:03:59.000Z".to_string(),
            sample_end: "2026-07-19T18:04:00.000Z".to_string(),
            sample_seconds: 1,
            upload_bytes_per_second: 2048.0,
            download_bytes_per_second: 4096.0,
            upload_bytes: 2048,
            download_bytes: 4096,
        }),
        accounts: Vec::new(),
    };

    let payload = serde_json::to_value(&snapshot).expect("serialize dashboard activity live");

    assert_eq!(payload["networkRealtimeRate"]["sampleSeconds"], 1);
    assert_eq!(payload["networkRealtimeRate"]["uploadBytes"], 2048);
    assert_eq!(
        payload["networkRealtimeRate"]["downloadBytesPerSecond"],
        4096.0
    );
}

#[test]
fn build_invocation_filters_normalizes_request_id() {
    let params = ListQuery {
        request_id: Some(" invoke-123 ".to_string()),
        ..Default::default()
    };

    let filters = build_invocation_filters(&params).expect("filters should build");

    assert_eq!(filters.request_id.as_deref(), Some("invoke-123"));
}

#[test]
fn build_invocation_filters_ignores_legacy_proxy_param() {
    let params = ListQuery {
        proxy: Some(" tokyo-edge-01 ".to_string()),
        ..Default::default()
    };

    let filters = build_invocation_filters(&params).expect("filters should build");

    assert_eq!(params.proxy.as_deref(), Some(" tokyo-edge-01 "));
    assert_eq!(filters.endpoint, None);
    assert_eq!(filters.request_id, None);
}

#[test]
fn response_body_falls_back_to_preview_when_complete() {
    let row = InvocationResponseBodyRow {
        id: 1,
        invoke_id: "invoke-preview".to_string(),
        payload: None,
        raw_response: "{\"error\":\"preview\"}".to_string(),
        request_raw_path: None,
        request_raw_size: None,
        request_raw_truncated: None,
        request_raw_truncated_reason: None,
        response_raw_path: None,
        response_raw_size: Some(19),
        response_raw_truncated: Some(0),
        response_raw_truncated_reason: None,
        detail_level: "full".to_string(),
        detail_prune_reason: None,
        response_content_encoding: None,
        failure_class: Some("service_failure".to_string()),
        upstream_request_id: None,
        attempt_public_id: None,
    };

    let (body, from_full_body) =
        resolve_response_body_text_from_row(&row, None).expect("preview should be reusable");

    assert_eq!(body, "{\"error\":\"preview\"}");
    assert!(!from_full_body);
}

#[test]
fn response_body_reports_detail_pruned_when_structured_only_preview_missing() {
    let row = InvocationResponseBodyRow {
        id: 2,
        invoke_id: "invoke-pruned".to_string(),
        payload: None,
        raw_response: String::new(),
        request_raw_path: None,
        request_raw_size: None,
        request_raw_truncated: None,
        request_raw_truncated_reason: None,
        response_raw_path: None,
        response_raw_size: None,
        response_raw_truncated: Some(0),
        response_raw_truncated_reason: None,
        detail_level: DETAIL_LEVEL_STRUCTURED_ONLY.to_string(),
        detail_prune_reason: Some("success_over_30d".to_string()),
        response_content_encoding: None,
        failure_class: Some("client_failure".to_string()),
        upstream_request_id: None,
        attempt_public_id: None,
    };

    let err = resolve_response_body_text_from_row(&row, None)
        .expect_err("structured-only rows should not expose a full body");

    assert_eq!(err, "detail_pruned");
}

#[test]
fn attempt_response_body_reports_attempt_specific_missing_reason() {
    let row = InvocationResponseBodyRow {
        id: 3,
        invoke_id: "invoke-attempt-missing".to_string(),
        payload: None,
        raw_response: String::new(),
        request_raw_path: None,
        request_raw_size: None,
        request_raw_truncated: None,
        request_raw_truncated_reason: None,
        response_raw_path: None,
        response_raw_size: None,
        response_raw_truncated: Some(0),
        response_raw_truncated_reason: None,
        detail_level: "full".to_string(),
        detail_prune_reason: None,
        response_content_encoding: None,
        failure_class: Some("service_failure".to_string()),
        upstream_request_id: None,
        attempt_public_id: Some("attempt-abc".to_string()),
    };

    assert_eq!(
        raw_response_fallback_reason(&row),
        "attempt_response_body_not_captured"
    );
}
