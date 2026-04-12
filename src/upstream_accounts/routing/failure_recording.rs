pub(crate) async fn record_pool_route_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    invoke_id: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    let request_started_at_iso = format_utc_iso(request_started_at_utc);
    let update_result = sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_selected_at = COALESCE(last_selected_at, ?3),
            last_error = NULL,
            last_error_at = NULL,
            last_route_failure_at = NULL,
            last_route_failure_kind = NULL,
            cooldown_until = NULL,
            consecutive_route_failures = 0,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?1
          AND (
                last_route_failure_at IS NULL
                OR last_route_failure_at <= ?4
            )
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(&now_iso)
    .bind(&request_started_at_iso)
    .execute(pool)
    .await?;
    if update_result.rows_affected() == 0 {
        return Ok(());
    }
    if let Some(sticky_key) = sticky_key {
        upsert_sticky_route(pool, sticky_key, account_id, &now_iso).await?;
    }
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED,
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: None,
            reason_message: None,
            http_status: None,
            failure_kind: None,
            invoke_id,
            sticky_key,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_pool_route_http_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    if route_http_failure_is_retryable_server_overloaded(status, error_message) {
        return record_pool_route_retryable_overload_failure(
            pool,
            account_id,
            sticky_key,
            error_message,
            invoke_id,
        )
        .await;
    }

    let classification = classify_pool_account_http_failure(account_kind, status, error_message);
    match classification.disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => {
            if let Some(sticky_key) = sticky_key {
                delete_sticky_route(pool, sticky_key).await?;
            }
            let now_iso = format_utc_iso(Utc::now());
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_error = ?3,
                    last_error_at = ?4,
                    last_route_failure_at = ?4,
                    last_route_failure_kind = ?5,
                    cooldown_until = NULL,
                    consecutive_route_failures = consecutive_route_failures + 1,
                    temporary_route_failure_streak_started_at = NULL,
                    updated_at = ?4
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(
                classification
                    .next_account_status
                    .unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR),
            )
            .bind(error_message)
            .bind(&now_iso)
            .bind(classification.failure_kind)
            .execute(pool)
            .await?;
            record_upstream_account_action(
                pool,
                account_id,
                UpstreamAccountActionPayload {
                    action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                    source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                    reason_code: Some(classification.reason_code),
                    reason_message: Some(error_message),
                    http_status: Some(status),
                    failure_kind: Some(classification.failure_kind),
                    invoke_id,
                    sticky_key,
                    occurred_at: &now_iso,
                },
            )
            .await?;
            Ok(())
        }
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => {
            let base_secs = if status == StatusCode::TOO_MANY_REQUESTS {
                15
            } else {
                5
            };
            apply_pool_route_cooldown_failure(
                pool,
                account_id,
                sticky_key,
                error_message,
                classification.failure_kind,
                classification.reason_code,
                status,
                base_secs,
                invoke_id,
            )
            .await
        }
    }
}

pub(crate) async fn record_pool_route_retryable_overload_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    apply_pool_route_cooldown_failure(
        pool,
        account_id,
        sticky_key,
        error_message,
        PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
        StatusCode::OK,
        5,
        invoke_id,
    )
    .await
}

pub(crate) async fn record_pool_route_transport_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    apply_pool_route_cooldown_failure(
        pool,
        account_id,
        sticky_key,
        error_message,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
        StatusCode::BAD_GATEWAY,
        5,
        invoke_id,
    )
    .await
}


pub(crate) async fn record_account_selected(pool: &Pool<Sqlite>, account_id: i64) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET last_selected_at = ?2,
            updated_at = ?2
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn record_compact_support_observation(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    if !matches!(
        status,
        COMPACT_SUPPORT_STATUS_SUPPORTED | COMPACT_SUPPORT_STATUS_UNSUPPORTED
    ) {
        return Ok(());
    }
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET compact_support_status = ?2,
            compact_support_observed_at = ?3,
            compact_support_reason = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(now_iso)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn apply_pool_route_cooldown_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: StatusCode,
    base_secs: i64,
    invoke_id: Option<&str>,
) -> Result<()> {
    let row = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    let now = Utc::now();
    let continuing_temporary_streak = row.consecutive_route_failures > 0
        && route_failure_kind_is_temporary(row.last_route_failure_kind.as_deref());
    let next_failures = if continuing_temporary_streak {
        row.consecutive_route_failures.max(0) + 1
    } else {
        1
    };
    let streak_started_at = if continuing_temporary_streak {
        row.temporary_route_failure_streak_started_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .or_else(|| {
                row.last_route_failure_at
                    .as_deref()
                    .and_then(parse_rfc3339_utc)
            })
            .unwrap_or(now)
    } else {
        now
    };
    let should_start_cooldown = next_failures >= POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD
        || now.signed_duration_since(streak_started_at).num_seconds()
            >= POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS;
    let exponent = (next_failures - 1).clamp(0, 5) as u32;
    let cooldown_secs =
        (base_secs * (1_i64 << exponent)).min(POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS);
    let now_iso = format_utc_iso(now);
    let streak_started_at_iso = format_utc_iso(streak_started_at);
    let cooldown_until =
        should_start_cooldown.then(|| format_utc_iso(now + ChronoDuration::seconds(cooldown_secs)));
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = ?3,
            last_error_at = ?4,
            last_route_failure_at = ?4,
            last_route_failure_kind = ?5,
            cooldown_until = ?6,
            consecutive_route_failures = ?7,
            temporary_route_failure_streak_started_at = ?8,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(error_message)
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(cooldown_until)
    .bind(next_failures)
    .bind(streak_started_at_iso)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: if should_start_cooldown {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED
            } else {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE
            },
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status: Some(http_status),
            failure_kind: Some(failure_kind),
            invoke_id,
            sticky_key,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}
