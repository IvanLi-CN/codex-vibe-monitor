use super::*;

pub(crate) async fn set_account_status(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    last_error: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = CASE
                WHEN ?2 = ?6 AND ?3 IS NULL THEN last_error
                ELSE ?3
            END,
            last_error_at = CASE
                WHEN ?2 = ?6 AND ?3 IS NULL THEN last_error_at
                WHEN ?3 IS NULL THEN last_error_at
                ELSE ?4
            END,
            last_route_failure_at = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE last_route_failure_at
            END,
            last_route_failure_kind = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE last_route_failure_kind
            END,
            cooldown_until = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE cooldown_until
            END,
            consecutive_route_failures = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN 0
                ELSE consecutive_route_failures
            END,
            temporary_route_failure_streak_started_at = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE temporary_route_failure_streak_started_at
            END,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(last_error)
    .bind(&now_iso)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(UPSTREAM_ACCOUNT_STATUS_SYNCING)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn mark_account_sync_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    route_state: SyncSuccessRouteState,
) -> Result<()> {
    mark_account_sync_success_with_proxy_snapshot(pool, account_id, source, route_state, None).await
}

pub(crate) async fn mark_account_sync_success_with_proxy_snapshot(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    route_state: SyncSuccessRouteState,
    proxy_snapshot: Option<&AccountMaintenanceProxySnapshot>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    match route_state {
        SyncSuccessRouteState::PreserveFailureState => {
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_synced_at = ?3,
                    last_successful_sync_at = ?3,
                    last_error = NULL,
                    last_error_at = NULL,
                    cooldown_until = CASE
                        WHEN last_action_source = ?4
                             AND last_action_reason_code IN (?5, ?6) THEN NULL
                        ELSE cooldown_until
                    END,
                    updated_at = ?3
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
            .bind(&now_iso)
            .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
            .bind("upstream_http_402")
            .bind("upstream_rejected")
            .execute(pool)
            .await?;
        }
        SyncSuccessRouteState::ClearFailureState => {
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_synced_at = ?3,
                    last_successful_sync_at = ?3,
                    last_error = NULL,
                    last_error_at = NULL,
                    last_route_failure_at = NULL,
                    last_route_failure_kind = NULL,
                    cooldown_until = NULL,
                    consecutive_route_failures = 0,
                    temporary_route_failure_streak_started_at = NULL,
                    updated_at = ?3
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
            .bind(&now_iso)
            .execute(pool)
            .await?;
        }
    }
    record_upstream_account_action_with_proxy_snapshot(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED,
            source,
            reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_OK),
            reason_message: None,
            http_status: None,
            failure_kind: None,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
        proxy_snapshot,
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_suppressed_sync_status_change_with_proxy_snapshot(
    pool: &Pool<Sqlite>,
    account_id: i64,
    restored_status: &str,
    source: &str,
    reason_code: &str,
    reason_message: &str,
    http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    proxy_snapshot: Option<&AccountMaintenanceProxySnapshot>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    let restored_status = if restored_status.trim().is_empty() {
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    } else {
        restored_status
    };
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(restored_status)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    record_status_change_suppressed_event_with_proxy_snapshot(
        pool,
        account_id,
        source,
        reason_code,
        reason_message,
        http_status,
        failure_kind,
        None,
        None,
        &now_iso,
        proxy_snapshot,
    )
    .await
}

pub(crate) async fn record_account_sync_recovery_blocked(
    pool: &Pool<Sqlite>,
    account_id: i64,
    restored_status: &str,
    source: &str,
    status: &str,
    reason_code: &'static str,
    reason_message: &str,
    preserved_error: Option<&str>,
    failure_kind: Option<&str>,
) -> Result<()> {
    if !account_status_change_reason_is_enabled(pool, account_id, reason_code).await? {
        return record_suppressed_sync_status_change_with_proxy_snapshot(
            pool,
            account_id,
            restored_status,
            source,
            reason_code,
            reason_message,
            None,
            failure_kind,
            None,
        )
        .await;
    }
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = COALESCE(?4, last_error),
            cooldown_until = CASE
                WHEN last_action_source = ?5
                     AND last_action_reason_code IN (?6, ?7) THEN NULL
                ELSE cooldown_until
            END,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(&now_iso)
    .bind(preserved_error)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
    .bind(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
    .bind(LEGACY_UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_REJECTED)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(reason_message),
            http_status: None,
            failure_kind,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_account_sync_hard_unavailable(
    pool: &Pool<Sqlite>,
    account_id: i64,
    restored_status: &str,
    source: &str,
    reason_code: &'static str,
    reason_message: &str,
    failure_kind: &'static str,
) -> Result<()> {
    if !account_status_change_reason_is_enabled(pool, account_id, reason_code).await? {
        return record_suppressed_sync_status_change_with_proxy_snapshot(
            pool,
            account_id,
            restored_status,
            source,
            reason_code,
            reason_message,
            None,
            Some(failure_kind),
            None,
        )
        .await;
    }
    let now_iso = format_utc_iso(Utc::now());
    let cooldown_until =
        maintenance_sync_rejected_cooldown_until(source, reason_code, reason_message, &now_iso);
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = ?4,
            last_error_at = ?3,
            last_route_failure_at = ?3,
            last_route_failure_kind = ?5,
            cooldown_until = ?6,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
    .bind(&now_iso)
    .bind(reason_message)
    .bind(failure_kind)
    .bind(cooldown_until)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(reason_message),
            http_status: None,
            failure_kind: Some(failure_kind),
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_account_sync_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    restored_status: &str,
    source: &str,
    status: &str,
    error_message: &str,
    reason_code: &'static str,
    http_status: Option<StatusCode>,
    failure_kind: &'static str,
    preserved_route_failure_kind: Option<&str>,
    clear_transient_route_failure_state: bool,
) -> Result<()> {
    record_account_sync_failure_with_proxy_snapshot(
        pool,
        account_id,
        restored_status,
        source,
        status,
        error_message,
        reason_code,
        http_status,
        failure_kind,
        preserved_route_failure_kind,
        clear_transient_route_failure_state,
        None,
    )
    .await
}

pub(crate) async fn record_account_sync_failure_with_proxy_snapshot(
    pool: &Pool<Sqlite>,
    account_id: i64,
    restored_status: &str,
    source: &str,
    status: &str,
    error_message: &str,
    reason_code: &'static str,
    http_status: Option<StatusCode>,
    failure_kind: &'static str,
    preserved_route_failure_kind: Option<&str>,
    clear_transient_route_failure_state: bool,
    proxy_snapshot: Option<&AccountMaintenanceProxySnapshot>,
) -> Result<()> {
    if !account_status_change_reason_is_enabled(pool, account_id, reason_code).await? {
        return record_suppressed_sync_status_change_with_proxy_snapshot(
            pool,
            account_id,
            restored_status,
            source,
            reason_code,
            error_message,
            http_status,
            Some(failure_kind),
            proxy_snapshot,
        )
        .await;
    }
    let now_iso = format_utc_iso(Utc::now());
    let cooldown_until =
        maintenance_sync_rejected_cooldown_until(source, reason_code, error_message, &now_iso);
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = ?4,
            last_error_at = ?3,
            last_route_failure_at = CASE
                WHEN ?6 = 1 AND ?5 IS NULL THEN NULL
                WHEN ?5 IS NULL THEN last_route_failure_at
                ELSE ?3
            END,
            last_route_failure_kind = CASE
                WHEN ?6 = 1 AND ?5 IS NULL THEN NULL
                ELSE COALESCE(?5, last_route_failure_kind)
            END,
            cooldown_until = CASE
                WHEN ?7 IS NOT NULL THEN ?7
                WHEN ?6 = 1 THEN NULL
                WHEN last_action_source = ?8
                     AND last_action_reason_code IN (?9, ?10) THEN NULL
                ELSE cooldown_until
            END,
            temporary_route_failure_streak_started_at = CASE
                WHEN ?6 = 1 THEN NULL
                ELSE temporary_route_failure_streak_started_at
            END,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(&now_iso)
    .bind(error_message)
    .bind(preserved_route_failure_kind)
    .bind(clear_transient_route_failure_state)
    .bind(cooldown_until)
    .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
    .bind("upstream_http_402")
    .bind("upstream_rejected")
    .execute(pool)
    .await?;
    record_upstream_account_action_with_proxy_snapshot(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status,
            failure_kind: Some(failure_kind),
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
        proxy_snapshot,
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_classified_account_sync_failure(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    restored_status: &str,
    source: &str,
    error_message: &str,
) -> Result<()> {
    record_classified_account_sync_failure_with_proxy_snapshot(
        pool,
        row,
        restored_status,
        source,
        error_message,
        None,
    )
    .await
}

pub(crate) async fn record_classified_account_sync_failure_with_proxy_snapshot(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    restored_status: &str,
    source: &str,
    error_message: &str,
    proxy_snapshot: Option<&AccountMaintenanceProxySnapshot>,
) -> Result<()> {
    let (disposition, reason_code, next_status, http_status, failure_kind) =
        classify_sync_failure(&row.kind, error_message);
    if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
        && disposition != UpstreamAccountFailureDisposition::HardUnavailable
    {
        return record_suppressed_sync_status_change_with_proxy_snapshot(
            pool,
            row.id,
            restored_status,
            source,
            reason_code,
            error_message,
            http_status,
            Some(failure_kind),
            proxy_snapshot,
        )
        .await;
    }
    let next_status = match disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => {
            next_status.unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR)
        }
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => UPSTREAM_ACCOUNT_STATUS_ACTIVE,
    };
    let (preserved_route_failure_kind, clear_transient_route_failure_state) = match disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => (Some(failure_kind), true),
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => (
            row.last_route_failure_kind.as_deref().filter(|_| {
                status_preserves_current_route_failure(&row.status)
                    && (upstream_account_quota_exhausted_state_is_current(
                        &row.status,
                        row.last_error_at.as_deref(),
                        row.last_route_failure_at.as_deref(),
                        row.last_route_failure_kind.as_deref(),
                        row.last_action_reason_code.as_deref(),
                    ) || route_failure_kind_is_temporary(
                        row.last_route_failure_kind.as_deref(),
                    ))
            }),
            false,
        ),
    };
    record_account_sync_failure_with_proxy_snapshot(
        pool,
        row.id,
        restored_status,
        source,
        next_status,
        error_message,
        reason_code,
        http_status,
        failure_kind,
        preserved_route_failure_kind,
        clear_transient_route_failure_state,
        proxy_snapshot,
    )
    .await
}
