use super::*;

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

    assert_sync_triggered_402_summary_and_detail(&pool, account_id).await;
}

async fn assert_sync_triggered_402_summary_and_detail(pool: &SqlitePool, account_id: i64) {
    let row = load_upstream_account_row(pool, account_id)
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
    assert_sync_triggered_402_detail(&detail, &row.cooldown_until);
}

fn assert_sync_triggered_402_detail(
    detail: &UpstreamAccountDetail,
    cooldown_until: &Option<String>,
) {
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
    assert_eq!(&detail.summary.cooldown_until, cooldown_until);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.reason_code.as_deref()),
        Some("upstream_http_402")
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.failure_kind.as_deref()),
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
