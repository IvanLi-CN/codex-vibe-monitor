#[test]
pub(crate) fn maintenance_plan_is_not_due_during_upstream_rejected_cooldown() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        42,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());

    assert!(
        !maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "active upstream-rejected cooldown should suppress maintenance scheduling"
    );
}

#[test]
pub(crate) fn maintenance_plan_prefers_explicit_upstream_rejected_cooldown_until() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        42,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.cooldown_until = Some("2026-04-13T17:00:00Z".to_string());

    assert!(
        !maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "explicit cooldown_until should be the canonical maintenance suppression signal"
    );
}

#[test]
pub(crate) fn maintenance_plan_is_due_when_reset_window_passes_even_during_upstream_rejected_cooldown()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        42,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-04-13T11:59:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.primary_resets_at = Some(format_utc_iso(now - ChronoDuration::minutes(1)));
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "reset-due accounts should bypass the temporary upstream-rejected cooldown"
    );
}

#[test]
pub(crate) fn maintenance_plan_ignores_wrapped_upstream_auth_errors_for_cooldown_blocking() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        42,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_error = Some(
        "oauth_upstream_rejected_request: pool upstream responded with 403: Forbidden".to_string(),
    );
    candidate.last_action_reason_code = Some("upstream_http_403".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH.to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "wrapped upstream auth errors should not enter the maintenance cooldown path"
    );
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_does_not_let_cooldown_blocked_accounts_consume_priority_slots()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };

    let mut cooldown_blocked = maintenance_candidates(
        1,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:58:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(5.0),
        Some(5.0),
    );
    cooldown_blocked.last_action_reason_code = Some("upstream_http_402".to_string());
    cooldown_blocked.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    cooldown_blocked.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    cooldown_blocked.last_action_at = Some("2026-04-13T11:58:00Z".to_string());

    let healthy_due = maintenance_candidates(
        2,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![cooldown_blocked, healthy_due],
        settings,
        Duration::from_secs(900),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].account_id, 2);
    assert_eq!(plans[0].tier, MaintenanceTier::Priority);
    assert_eq!(
        plans[0].sync_interval_secs,
        settings.primary_sync_interval_secs
    );
}

#[test]
pub(crate) fn maintenance_plan_is_due_for_call_driven_upstream_rejected_errors() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        77,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_error = Some("deactivated_workspace".to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "ordinary routed 402s should still allow maintenance to retry promptly"
    );
}

#[test]
pub(crate) fn maintenance_plan_is_due_for_generic_403_upstream_rejected_text() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        88,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:55:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:55:00Z".to_string());
    candidate.last_action_reason_code = Some("upstream_http_403".to_string());
    candidate.last_route_failure_at = Some("2026-04-13T11:55:00Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH.to_string());
    candidate.last_error = Some(
        "usage endpoint returned 403 Forbidden: upstream rejected request by policy".to_string(),
    );

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "generic 403 text that happens to mention upstream rejected should not trigger the 402-only maintenance cooldown"
    );
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_keeps_refresh_due_accounts_on_primary_cadence()
{
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 100,
    };

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![maintenance_candidates(
            7,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:59:00Z"),
            None,
            Some("2026-03-23T12:10:00Z"),
            Some(6.0),
            Some(6.0),
        )],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert!(
        plans.is_empty(),
        "refresh-due accounts should stay on the configured primary cadence until the interval elapses"
    );
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_routes_working_accounts_to_high_frequency() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        8,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
    assert_eq!(plans[0].sync_interval_secs, 60);
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_keeps_working_refresh_due_accounts_high_frequency()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        81,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-03-23T12:10:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
    assert_eq!(plans[0].sync_interval_secs, 60);
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_routes_degraded_accounts_to_high_frequency() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        9,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        Some("2026-03-23T11:59:45Z"),
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_route_failure_at = Some("2026-03-23T11:59:45Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE.to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
    assert_eq!(plans[0].sync_interval_secs, 60);
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_keeps_credits_exhausted_accounts_out_of_high_frequency()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        91,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());
    candidate.credits_has_credits = Some(1);
    candidate.credits_unlimited = Some(0);
    candidate.credits_balance = Some("0".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 0);
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_triggers_reset_due_sync_before_interval() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 100,
    };
    let mut candidate = maintenance_candidates(
        10,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::Priority);
    assert_eq!(plans[0].sync_interval_secs, 300);
}

#[test]
pub(crate) fn maintenance_reset_due_only_triggers_once_per_reset_boundary() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut before_reset_sync = maintenance_candidates(
        11,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    before_reset_sync.secondary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    assert!(maintenance_reset_due(&before_reset_sync, now));

    let mut after_reset_sync = before_reset_sync.clone();
    after_reset_sync.last_synced_at = Some("2026-03-23T11:59:30Z".to_string());
    assert!(!maintenance_reset_due(&after_reset_sync, now));
}

#[test]
pub(crate) fn maintenance_reset_due_stops_after_post_reset_failure_even_if_status_stays_active() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        12,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    assert!(maintenance_reset_due(&candidate, now));

    candidate.last_synced_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_error_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_route_failure_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE.to_string());

    assert!(!maintenance_reset_due(&candidate, now));
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_preserves_primary_cadence_after_call_driven_error()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 100,
    };
    let mut candidate = maintenance_candidates(
        13,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-03-23T11:50:00Z"),
        Some("2026-03-23T11:58:30Z"),
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
    candidate.last_action_at = Some("2026-03-23T11:58:30Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert!(
        plans.is_empty(),
        "call-driven error transitions should still honor the configured primary cadence"
    );
}

#[test]
pub(crate) fn maintenance_reset_due_ignores_call_driven_error_after_reset() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        14,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-03-23T11:58:30Z"),
        Some("2026-03-23T11:59:20Z"),
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());

    assert!(
        maintenance_reset_due(&candidate, now),
        "call-driven failures should not consume the post-reset catch-up sync"
    );
}

#[test]
pub(crate) fn maintenance_reset_due_ignores_deferred_egress_throttle_after_reset() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        15,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

    assert!(
        maintenance_reset_due(&candidate, now),
        "egress-deferred maintenance should not consume the post-reset catch-up sync"
    );
}

#[test]
pub(crate) fn maintenance_interval_is_due_respects_deferred_egress_throttle_anchor() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        16,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:00:00Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

    assert!(
        !maintenance_interval_is_due(&candidate, 300, now),
        "ordinary maintenance should still use egress-deferred actions as the retry anchor"
    );
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_requeues_reset_due_after_deferred_egress_throttle()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 100,
    };
    let mut candidate = maintenance_candidates(
        17,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::Priority);
    assert_eq!(plans[0].sync_interval_secs, 300);
}

pub(crate) fn test_routing_candidate(id: i64) -> AccountRoutingCandidateRow {
    AccountRoutingCandidateRow {
        id,
        plan_type: None,
        secondary_used_percent: None,
        secondary_window_minutes: None,
        secondary_resets_at: None,
        primary_used_percent: None,
        primary_window_minutes: None,
        primary_resets_at: None,
        local_primary_limit: None,
        local_secondary_limit: None,
        credits_has_credits: None,
        credits_unlimited: None,
        credits_balance: None,
        last_selected_at: Some("2026-03-23T11:00:00Z".to_string()),
        active_sticky_conversations: 0,
        in_flight_reservations: 0,
    }
}

#[test]
pub(crate) fn compare_routing_candidates_prefers_short_window_that_is_about_to_reset() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 26, 7, 0, 0)
        .single()
        .expect("valid now");
    let mut short_reset_soon = test_routing_candidate(1);
    short_reset_soon.plan_type = Some("team".to_string());
    short_reset_soon.primary_used_percent = Some(70.0);
    short_reset_soon.primary_window_minutes = Some(300);
    short_reset_soon.primary_resets_at = Some(format_utc_iso(now + ChronoDuration::minutes(5)));
    short_reset_soon.secondary_used_percent = Some(40.0);
    short_reset_soon.secondary_window_minutes = Some(7 * 24 * 60);
    short_reset_soon.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(1)));

    let mut long_only = test_routing_candidate(2);
    long_only.plan_type = Some("free".to_string());
    long_only.secondary_used_percent = Some(30.0);
    long_only.secondary_window_minutes = Some(7 * 24 * 60);
    long_only.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(6)));

    assert_eq!(
        compare_routing_candidates_at(&short_reset_soon, &long_only, now),
        std::cmp::Ordering::Less,
        "a short-window account that is about to reset should beat a lower-used long-only account whose reset is still far away",
    );
}

#[test]
pub(crate) fn compare_routing_candidates_penalizes_far_from_reset_pressure() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 26, 7, 0, 0)
        .single()
        .expect("valid now");
    let mut stretched_team = test_routing_candidate(1);
    stretched_team.plan_type = Some("team".to_string());
    stretched_team.primary_used_percent = Some(95.0);
    stretched_team.primary_window_minutes = Some(300);
    stretched_team.primary_resets_at = Some(format_utc_iso(now + ChronoDuration::minutes(250)));
    stretched_team.secondary_used_percent = Some(80.0);
    stretched_team.secondary_window_minutes = Some(7 * 24 * 60);
    stretched_team.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(6)));

    let mut healthier_long = test_routing_candidate(2);
    healthier_long.plan_type = Some("free".to_string());
    healthier_long.secondary_used_percent = Some(20.0);
    healthier_long.secondary_window_minutes = Some(7 * 24 * 60);
    healthier_long.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(3)));

    assert_eq!(
        compare_routing_candidates_at(&stretched_team, &healthier_long, now),
        std::cmp::Ordering::Greater,
        "near-exhausted windows with lots of time left should sort behind healthier long-window accounts",
    );
}

#[test]
pub(crate) fn compare_routing_candidates_treats_zero_percent_single_window_as_limited() {
    let mut single_window = test_routing_candidate(1);
    single_window.primary_used_percent = Some(0.0);
    single_window.primary_window_minutes = Some(7 * 24 * 60);
    single_window.active_sticky_conversations = 2;

    let unlimited = test_routing_candidate(2);

    assert_eq!(
        compare_routing_candidates(&single_window, &unlimited),
        std::cmp::Ordering::Greater,
        "a single remote window sample, even at 0%, should still participate in the tighter long-window load caps",
    );
}

#[test]
pub(crate) fn candidate_capacity_profile_tightens_for_long_only_accounts() {
    let mut long_only = test_routing_candidate(1);
    long_only.secondary_used_percent = Some(10.0);
    long_only.secondary_window_minutes = Some(7 * 24 * 60);
    let mut short_window = test_routing_candidate(2);
    short_window.primary_used_percent = Some(10.0);
    short_window.primary_window_minutes = Some(300);

    let long_only_capacity = long_only.capacity_profile();
    let short_window_capacity = short_window.capacity_profile();

    assert_eq!(long_only_capacity.soft_limit, 1);
    assert_eq!(long_only_capacity.hard_cap, 2);
    assert_eq!(short_window_capacity.soft_limit, 2);
    assert_eq!(short_window_capacity.hard_cap, 3);
}

#[test]
pub(crate) fn candidate_capacity_profile_preserves_legacy_limit_signals_without_window_metadata() {
    let mut legacy_long_only = test_routing_candidate(1);
    legacy_long_only.secondary_used_percent = Some(10.0);

    let mut locally_limited = test_routing_candidate(2);
    locally_limited.local_secondary_limit = Some(100.0);

    let legacy_capacity = legacy_long_only.capacity_profile();
    let local_capacity = locally_limited.capacity_profile();

    assert_eq!(legacy_capacity.soft_limit, 1);
    assert_eq!(legacy_capacity.hard_cap, 2);
    assert_eq!(local_capacity.soft_limit, 1);
    assert_eq!(local_capacity.hard_cap, 2);
}

#[test]
pub(crate) fn derive_work_status_only_counts_last_selected_within_five_minute_window() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 1, 12, 0, 0)
        .single()
        .expect("valid now");
    let recent_selected =
        format_utc_iso(now - ChronoDuration::minutes(4) - ChronoDuration::seconds(59));
    let stale_selected =
        format_utc_iso(now - ChronoDuration::minutes(5) - ChronoDuration::seconds(1));

    assert_eq!(
        derive_upstream_account_work_status(
            true,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&recent_selected),
            now,
        ),
        UPSTREAM_ACCOUNT_WORK_STATUS_WORKING
    );
    assert_eq!(
        derive_upstream_account_work_status(
            true,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&stale_selected),
            now,
        ),
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE
    );
}

#[test]
pub(crate) fn normalize_concurrency_limit_rejects_values_outside_supported_range() {
    assert_eq!(
        normalize_concurrency_limit(Some(-1), "concurrencyLimit"),
        Err((
            StatusCode::BAD_REQUEST,
            "concurrencyLimit must be between 0 and 30".to_string(),
        ))
    );
    assert_eq!(
        normalize_concurrency_limit(Some(31), "concurrencyLimit"),
        Err((
            StatusCode::BAD_REQUEST,
            "concurrencyLimit must be between 0 and 30".to_string(),
        ))
    );
    assert_eq!(normalize_concurrency_limit(None, "concurrencyLimit"), Ok(0));
    assert_eq!(
        normalize_concurrency_limit(Some(30), "concurrencyLimit"),
        Ok(30)
    );
}

#[test]
pub(crate) fn build_effective_routing_rule_uses_smallest_non_zero_concurrency_limit() {
    let tags = vec![
        test_account_tag_summary(1, "unlimited", 0),
        test_account_tag_summary(2, "soft", 6),
        test_account_tag_summary(3, "strict", 3),
    ];

    let rule = build_effective_routing_rule(&tags);

    assert_eq!(rule.concurrency_limit, 3);
    assert_eq!(rule.source_tag_ids, vec![1, 2, 3]);
    assert_eq!(
        rule.source_tag_names,
        vec![
            "unlimited".to_string(),
            "soft".to_string(),
            "strict".to_string(),
        ]
    );
}

#[test]
pub(crate) fn normalize_tag_priority_tier_defaults_to_normal_and_rejects_invalid_values() {
    assert_eq!(
        normalize_tag_priority_tier(None),
        Ok(TagPriorityTier::Normal)
    );
    assert_eq!(
        normalize_tag_priority_tier(Some("primary")),
        Ok(TagPriorityTier::Primary)
    );
    assert_eq!(
        normalize_tag_priority_tier(Some("fallback")),
        Ok(TagPriorityTier::Fallback)
    );
    assert_eq!(
        normalize_tag_priority_tier(Some("unexpected")),
        Err((
            StatusCode::BAD_REQUEST,
            "priorityTier must be one of: primary, normal, fallback, no_new".to_string(),
        ))
    );
}

use super::*;
