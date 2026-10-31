use super::*;

#[test]
fn resolve_due_maintenance_dispatch_plans_schedules_refresh_due_accounts_on_one_minute_cadence() {
    assert_eq!(DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS, 5 * 60);
    assert_eq!(UPSTREAM_ACCOUNT_MAINTENANCE_TICK_SECS, 60);

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
            Some("2026-03-23T12:05:00Z"),
            Some(6.0),
            Some(6.0),
        )],
        settings,
        Duration::from_secs(5 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::Priority);
    assert_eq!(plans[0].sync_interval_secs, 60);
}

#[test]
fn resolve_due_maintenance_dispatch_plans_does_not_refresh_before_five_minute_window() {
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
            70,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:59:00Z"),
            None,
            Some("2026-03-23T12:06:00Z"),
            Some(6.0),
            Some(6.0),
        )],
        settings,
        Duration::from_secs(DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS),
        now,
    );

    assert!(
        plans.is_empty(),
        "accounts outside the five-minute refresh window should keep the normal sync cadence"
    );
}

#[test]
fn maintenance_refresh_due_uses_eight_day_fallback_without_parseable_expiration() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let refresh_lead_time = Duration::from_secs(DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS);
    let mut candidate = maintenance_candidates(
        71,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:59:00Z"),
        None,
        None,
        Some(6.0),
        Some(6.0),
    );
    candidate.last_refreshed_at = Some("2026-03-15T11:59:59Z".to_string());
    assert!(maintenance_refresh_due(&candidate, refresh_lead_time, now));

    candidate.last_refreshed_at = Some("2026-03-15T12:00:01Z".to_string());
    assert!(!maintenance_refresh_due(&candidate, refresh_lead_time, now));
}
