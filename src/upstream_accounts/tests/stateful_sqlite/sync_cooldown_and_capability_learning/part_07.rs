#[tokio::test]
pub(crate) async fn oauth_sync_retry_after_refresh_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-quota-preserving-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "retry-quota-preserve@example.com",
                    Some("org_retry_quota_preserve"),
                    Some("user_retry_quota_preserve"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Quota Preserve OAuth",
        "retry-quota-preserve@example.com",
        "org_retry_quota_preserve",
        "user_retry_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after gateway failure")
        .expect("oauth row exists after gateway failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_error.as_deref(),
        Some("usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable")
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    assert_quota_marker_summary_and_detail(&state.pool, account_id, after).await;

    assert_retry_request_counts(&usage_requests, &token_requests);
    server.abort();
}

async fn assert_quota_marker_summary_and_detail(
    pool: &SqlitePool,
    account_id: i64,
    after: UpstreamAccountRow,
) {
    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    let detail = load_upstream_account_detail(pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

fn assert_retry_request_counts(usage_requests: &AtomicUsize, token_requests: &AtomicUsize) {
    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
pub(crate) async fn oauth_sync_refresh_failure_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![(
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": {
                        "message": "Session cookie expired during usage snapshot"
                    }
                }),
            )],
            json!({
                "unexpected": "shape"
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Refresh Failure Quota Preserve OAuth",
        "refresh-quota-preserve@example.com",
        "org_refresh_quota_preserve",
        "user_refresh_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after refresh failure")
        .expect("oauth row exists after refresh failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(after.last_action_http_status, None);
    assert!(
        after
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("failed to decode OAuth token response"))
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    assert_quota_marker_summary_and_detail(&state.pool, account_id, after).await;

    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_direct_fetch_failure_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, server) = spawn_usage_snapshot_server(
        StatusCode::BAD_GATEWAY,
        json!({
            "error": {
                "message": "gateway temporarily unavailable"
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&usage_base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Direct Failure Quota Preserve OAuth",
        "direct-quota-preserve@example.com",
        "org_direct_quota_preserve",
        "user_direct_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after direct fetch failure")
        .expect("oauth row exists after direct fetch failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert!(
        after
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("502 Bad Gateway"))
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    assert_quota_marker_summary_and_detail(&state.pool, account_id, after).await;
    server.abort();
}

#[tokio::test]
pub(crate) async fn classified_sync_failure_preserves_existing_route_cooldown_across_new_error_timestamp()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Preserved Cooldown OAuth").await;
    let previous_failure_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(2));
    let cooldown_until = format_utc_iso(Utc::now() + ChronoDuration::minutes(5));

    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = ?6,
                consecutive_route_failures = 1,
                last_action = ?7,
                last_action_source = ?8,
                last_action_reason_code = ?9,
                last_action_reason_message = ?3,
                last_action_http_status = ?10,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind("seed preserved cooldown")
    .bind(&previous_failure_at)
    .bind(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    .bind(&cooldown_until)
    .bind(UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
    .bind(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
    .bind(429)
    .execute(&pool)
    .await
    .expect("seed preserved cooldown row");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load seeded cooldown row")
        .expect("seeded cooldown row exists");
    record_classified_account_sync_failure(
        &pool,
        &row,
        row.status.as_str(),
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable",
    )
    .await
    .expect("record classified retry failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load cooldown row after retry failure")
        .expect("cooldown row after retry failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);
    assert_ne!(
        after.last_route_failure_at.as_deref(),
        Some(previous_failure_at.as_str())
    );

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

#[tokio::test]
pub(crate) async fn classified_sync_hard_unavailable_replaces_stale_quota_marker_from_current_syncing_row()
 {
    classified_sync_hard_unavailable_replaces_stale_quota_marker_from_current_syncing_row_impl()
        .await;
}

async fn classified_sync_hard_unavailable_replaces_stale_quota_marker_from_current_syncing_row_impl()
 {
    let pool = test_pool().await;

    for (reason_code, http_status, failure_kind, error_message) in [
        (
            "upstream_http_401",
            StatusCode::UNAUTHORIZED,
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            "usage endpoint returned 401 Unauthorized: Missing scopes: api.responses.write",
        ),
        (
            "upstream_http_402",
            StatusCode::PAYMENT_REQUIRED,
            PROXY_FAILURE_UPSTREAM_HTTP_402,
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
        ),
        (
            "upstream_http_403",
            StatusCode::FORBIDDEN,
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            "usage endpoint returned 403 Forbidden: You have insufficient permissions for this operation.",
        ),
    ] {
        assert_hard_unavailable_sync_case(
            &pool,
            reason_code,
            http_status,
            failure_kind,
            error_message,
        )
        .await;
    }
}

async fn assert_hard_unavailable_sync_case(
    pool: &SqlitePool,
    reason_code: &str,
    http_status: StatusCode,
    failure_kind: &str,
    error_message: &str,
) {
    let account_id =
        insert_oauth_account(pool, &format!("Syncing hard unavailable {reason_code}")).await;
    seed_hard_unavailable_route_failure(
        pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    set_account_status(pool, account_id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None)
        .await
        .expect("mark row syncing");
    let current_row = load_upstream_account_row(pool, account_id)
        .await
        .expect("load current syncing row")
        .expect("current syncing row exists");
    assert_eq!(current_row.status, UPSTREAM_ACCOUNT_STATUS_SYNCING);
    record_classified_account_sync_failure(
        pool,
        &current_row,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        error_message,
    )
    .await
    .expect("record hard unavailable failure against syncing row");
    let after = load_upstream_account_row(pool, account_id)
        .await
        .expect("load syncing row after hard unavailable failure")
        .expect("syncing row after hard unavailable failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(after.last_action_reason_code.as_deref(), Some(reason_code));
    assert_eq!(
        after.last_action_http_status,
        Some(http_status.as_u16() as i64)
    );
    assert_eq!(after.last_route_failure_kind.as_deref(), Some(failure_kind));
    assert_eq!(after.last_route_failure_at, after.last_error_at);
    if reason_code == "upstream_http_402" {
        let cooldown_until = after
            .cooldown_until
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .expect("maintenance-triggered 402 should write explicit cooldown");
        let failed_at = after
            .last_action_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .expect("maintenance-triggered 402 should record last_action_at");
        assert_eq!(
            cooldown_until - failed_at,
            ChronoDuration::seconds(UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS)
        );
    } else {
        assert_eq!(after.cooldown_until, None);
    }
    assert_eq!(after.temporary_route_failure_streak_started_at, None);
    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
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
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

#[tokio::test]
pub(crate) async fn classified_sync_wrapped_upstream_rejected_permission_keeps_existing_cooldown_policy()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Wrapped upstream rejected cooldown").await;

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load fresh row")
        .expect("fresh row exists");
    record_classified_account_sync_failure(
        &pool,
        &row,
        row.status.as_str(),
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "oauth_upstream_rejected_request: pool upstream responded with 403: Forbidden",
    )
    .await
    .expect("record wrapped upstream rejected sync failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after wrapped upstream rejected sync failure")
        .expect("row after wrapped upstream rejected sync failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_403")
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
    );
    assert_eq!(
        after.cooldown_until, None,
        "wrapped upstream auth errors should keep the old no-cooldown behavior"
    );
}

#[tokio::test]
pub(crate) async fn classified_sync_failure_emits_suppressed_event_when_reason_toggle_disabled() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Suppressed Sync 402").await;
    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_status_change_upstream_http_402 = 0 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("disable sync 402 status change toggle");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load fresh row")
        .expect("fresh row exists");
    record_classified_account_sync_failure(
        &pool,
        &row,
        row.status.as_str(),
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
    )
    .await
    .expect("record suppressed sync failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after suppressed sync failure")
        .expect("row after suppressed sync failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(after.last_error, None);
    assert_eq!(after.last_action, None);
    assert_eq!(after.last_route_failure_kind, None);
    assert_eq!(after.cooldown_until, None);

    let detail = load_upstream_account_detail(&pool, account_id)
        .await
        .expect("load suppressed sync detail")
        .expect("suppressed sync detail exists");
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
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE
    );
    let event = detail
        .recent_actions
        .first()
        .expect("suppressed sync event should be recorded");
    assert_eq!(
        event.action,
        UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED
    );
    assert_eq!(
        event.source,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE
    );
    assert_eq!(
        event.reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
    );
    assert_eq!(event.http_status, Some(402));
    assert_eq!(
        event.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
    );
    assert!(
        event
            .reason_message
            .as_deref()
            .is_some_and(|value| value.contains("402 Payment Required"))
    );
}

#[tokio::test]
pub(crate) async fn classified_sync_non_rejected_failure_clears_existing_maintenance_rejected_cooldown()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Rejected Cooldown Replaced").await;

    record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("seed maintenance rejected cooldown");

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before replacement failure")
        .expect("row exists before replacement failure");
    assert!(before.cooldown_until.is_some());

    record_classified_account_sync_failure(
            &pool,
            &before,
            before.status.as_str(),
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "usage endpoint returned 403 Forbidden: You have insufficient permissions for this operation.",
        )
        .await
        .expect("record replacement sync failure");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after replacement failure")
        .expect("row exists after replacement failure");
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_403")
    );
    assert_eq!(after.cooldown_until, None);
}

#[tokio::test]
pub(crate) async fn mark_account_sync_success_clears_explicit_maintenance_upstream_rejected_cooldown()
 {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Rejected Cooldown Success").await;

    record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("seed maintenance rejected cooldown");

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before success")
        .expect("row exists before success");
    assert!(before.cooldown_until.is_some());

    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark sync success");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after success")
        .expect("row exists after success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_402),
        "preserve-failure success should keep the last route failure marker while clearing the explicit maintenance cooldown"
    );
}

#[tokio::test]
pub(crate) async fn classified_sync_failure_preserves_quota_marker_from_current_syncing_row() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Quota Syncing Preserve").await;

    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    set_account_status(&pool, account_id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None)
        .await
        .expect("mark row syncing");

    let current_row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load current syncing row")
        .expect("current syncing row exists");
    assert_eq!(current_row.status, UPSTREAM_ACCOUNT_STATUS_SYNCING);

    record_classified_account_sync_failure(
        &pool,
        &current_row,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        "usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable",
    )
    .await
    .expect("record retry failure against syncing row");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load syncing row after retry failure")
        .expect("syncing row after retry failure exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}

#[tokio::test]
pub(crate) async fn oauth_sync_proactively_quarantines_snapshot_exhausted_account_without_prior_route_failure()
 {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Sync Snapshot Exhausted",
        "snapshot-exhausted@example.com",
        "org_snapshot_exhausted",
        "user_snapshot_exhausted",
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after proactive quarantine")
        .expect("oauth row exists after proactive quarantine");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
pub(crate) async fn resolver_short_circuits_when_only_persisted_snapshot_exhausted_accounts_remain()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let first = insert_api_key_account(&state.pool, "Exhausted A").await;
    let second = insert_api_key_account(&state.pool, "Exhausted B").await;
    let third = insert_api_key_account(&state.pool, "Exhausted C").await;
    let now_iso = format_utc_iso(Utc::now());
    for account_id in [first, second, third] {
        insert_limit_sample_with_usage(&state.pool, account_id, &now_iso, Some(100.0), Some(40.0))
            .await;
    }

    let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
        .await
        .expect("resolve pool account");
    assert!(matches!(resolution, PoolAccountResolution::RateLimited));
}

use super::*;
