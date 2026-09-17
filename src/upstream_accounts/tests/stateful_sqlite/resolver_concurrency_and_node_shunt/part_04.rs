#[test]
pub(crate) fn classify_pool_account_http_failure_treats_usage_limit_reached_as_quota_exhausted() {
    let classification = classify_pool_account_http_failure(
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        StatusCode::TOO_MANY_REQUESTS,
        "pool upstream responded with 429: The usage limit has been reached",
    );

    assert_eq!(
        classification.disposition,
        UpstreamAccountFailureDisposition::RateLimited
    );
    assert_eq!(
        classification.reason_code,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
    );
    assert_eq!(
        classification.failure_kind,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
    );
}

#[tokio::test]
pub(crate) async fn quota_exhausted_oauth_summary_and_detail_export_as_rate_limited() {
    quota_exhausted_oauth_summary_and_detail_export_as_rate_limited_impl().await;
}

async fn quota_exhausted_oauth_summary_and_detail_export_as_rate_limited_impl() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Quota Exhausted OAuth").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            Some("sticky-quota-exhausted"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_quota_exhausted"),
        )
        .await
        .expect("record wrapped 429 route failure");

    let route_failure_row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load route failure row")
        .expect("route failure row exists");
    record_account_sync_recovery_blocked(
        &pool,
        account_id,
        &route_failure_row.status,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        &route_failure_row.status,
        UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
        "latest usage snapshot still shows an exhausted upstream usage limit window",
        route_failure_row.last_error.as_deref(),
        route_failure_row.last_route_failure_kind.as_deref(),
    )
    .await
    .expect("record blocked sync recovery");

    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                100.0, 300, ?3,
                64.0, 10080, ?4,
                1, 0, '0.00'
            )
            "#,
    )
    .bind(account_id)
    .bind("2026-03-24T18:00:27Z")
    .bind("2026-03-30T16:06:33Z")
    .bind("2026-04-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("insert exhausted usage sample");

    assert_quota_exhausted_summary(&pool, account_id).await;
    assert_quota_exhausted_detail(&pool, account_id).await;
}

async fn assert_quota_exhausted_summary(pool: &SqlitePool, account_id: i64) {
    let row = load_upstream_account_row(pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    let latest = load_latest_usage_sample(pool, account_id)
        .await
        .expect("load latest usage sample");
    let summary = build_summary_from_row(
        &row,
        latest.as_ref(),
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(
        summary.last_error.as_deref(),
        Some(
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached"
        )
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        summary
            .primary_window
            .as_ref()
            .map(|window| window.used_percent),
        Some(100.0)
    );
    assert_eq!(
        summary
            .primary_window
            .as_ref()
            .and_then(|window| window.resets_at.as_deref()),
        Some("2026-03-30T16:06:33Z")
    );
}

async fn assert_quota_exhausted_detail(pool: &SqlitePool, account_id: i64) {
    let detail = load_upstream_account_detail(pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.health_status,
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(
        detail.summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    let action = detail.recent_actions.first().expect("sync block action");
    assert_eq!(action.action, UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED);
    assert_eq!(
        action.reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        action.failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
}

#[tokio::test]
pub(crate) async fn sync_triggered_402_summary_and_detail_export_as_upstream_rejected() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Workspace Sync Blocked OAuth").await;

    record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("record sync-triggered 402 hard unavailable");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load sync-triggered 402 row")
        .expect("sync-triggered 402 row exists");
    let cooldown_until = row
        .cooldown_until
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .expect("maintenance-triggered 402 should write explicit cooldown");
    let failed_at = row
        .last_action_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .expect("sync-triggered 402 should record last_action_at");
    assert_eq!(
        cooldown_until - failed_at,
        ChronoDuration::seconds(UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS,)
    );
    assert_sync_triggered_402_exports(&pool, account_id, row).await;
}

async fn assert_sync_triggered_402_exports(
    pool: &SqlitePool,
    account_id: i64,
    row: UpstreamAccountRow,
) {
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(
        summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        summary.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some("upstream_http_402")
    );
    assert_eq!(
        summary.last_error.as_deref(),
        Some(
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}"
        )
    );
    assert_eq!(summary.cooldown_until, row.cooldown_until);

    let detail = load_upstream_account_detail(pool, account_id)
        .await
        .expect("load sync-triggered 402 detail")
        .expect("sync-triggered 402 detail exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        detail.summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        detail.summary.last_action_reason_code.as_deref(),
        Some("upstream_http_402")
    );
    assert_eq!(detail.summary.cooldown_until, row.cooldown_until);
    let action = detail
        .recent_actions
        .first()
        .expect("sync hard unavailable action");
    assert_eq!(action.action, UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE);
    assert_eq!(action.reason_code.as_deref(), Some("upstream_http_402"));
    assert_eq!(
        action.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
    );
}

#[tokio::test]
pub(crate) async fn stale_quota_route_failure_does_not_hide_newer_sync_error() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Stale quota marker OAuth").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            Some("sticky-stale-quota"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_stale_quota"),
        )
        .await
        .expect("record stale wrapped 429 route failure");

    record_account_sync_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        "usage snapshot parse error after refresh",
        UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR,
        None,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        None,
        false,
    )
    .await
    .expect("record newer sync failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
    );
    assert_eq!(
        summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
    );
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );

    let detail = load_upstream_account_detail(&pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
    );
    assert_eq!(
        detail.summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
}

#[tokio::test]
pub(crate) async fn stale_quota_route_failure_does_not_hide_newer_sync_402_error() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Stale quota marker 402 OAuth").await;

    record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            false,
            Some("sticky-stale-quota"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_stale_quota"),
        )
        .await
        .expect("record stale wrapped 429 route failure");

    record_account_sync_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            "upstream_http_402",
            Some(StatusCode::PAYMENT_REQUIRED),
            PROXY_FAILURE_UPSTREAM_HTTP_402,
            None,
            false,
        )
        .await
        .expect("record legacy-style 402 sync failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load updated 402 row")
        .expect("updated 402 row exists");
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );

    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some("upstream_http_402")
    );

    let detail = load_upstream_account_detail(&pool, account_id)
        .await
        .expect("load updated 402 detail")
        .expect("updated 402 detail exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        detail.summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        detail.summary.last_action_reason_code.as_deref(),
        Some("upstream_http_402")
    );
}

#[tokio::test]
pub(crate) async fn blocked_api_key_manual_recovery_does_not_export_as_active_rate_limited() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Manual Recovery API Key").await;

    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    record_account_sync_recovery_blocked(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED,
            "manual recovery required because API key sync cannot verify whether the upstream usage limit has reset",
            Some("seed hard unavailable"),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED),
        )
        .await
        .expect("record blocked recovery");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load blocked api key row")
        .expect("blocked api key row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
    );
    assert_eq!(
        summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
    );
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
    );

    let detail = load_upstream_account_detail(&pool, account_id)
        .await
        .expect("load blocked api key detail")
        .expect("blocked api key detail exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
    );
    assert_eq!(
        detail.summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(
        detail.summary.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.reason_code.as_deref()),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
    );
}

#[tokio::test]
pub(crate) async fn explicit_reauth_phrase_without_reauth_reason_does_not_force_needs_reauth() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "API key rejected wording").await;
    let now_iso = format_utc_iso(Utc::now());

    sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_action = ?5,
                last_action_source = ?6,
                last_action_reason_code = ?7,
                last_action_reason_message = ?3,
                last_action_http_status = ?8,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .bind(
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
        )
        .bind(&now_iso)
        .bind(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
        .bind("upstream_http_403")
        .bind(403)
        .execute(&pool)
        .await
        .expect("seed non-reauth rejection state");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert_ne!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_ne!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

#[tokio::test]
pub(crate) async fn legacy_oauth_explicit_reauth_error_without_reason_code_still_exports_needs_reauth()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Legacy OAuth Reauth").await;
    let now_iso = format_utc_iso(Utc::now());

    sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                last_action = ?6,
                last_action_source = ?7,
                last_action_reason_code = NULL,
                last_action_reason_message = ?3,
                last_action_http_status = ?8,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .bind(
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
        )
        .bind(&now_iso)
        .bind(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
        .bind(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
        .bind(403)
        .execute(&pool)
        .await
        .expect("seed legacy oauth reauth state");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load updated row")
        .expect("updated row exists");
    let summary = build_summary_from_row(
        &row,
        None,
        row.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );

    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

use super::*;
