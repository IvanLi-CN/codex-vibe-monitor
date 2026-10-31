use super::*;

#[derive(Debug, Clone, FromRow)]
struct UpstreamAccountRefreshStateRow {
    account_id: i64,
    operation: String,
    state: String,
    last_attempt_at: Option<String>,
    last_success_at: Option<String>,
    next_attempt_at: Option<String>,
    source: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct UpstreamAccountRefreshHistoryRow {
    id: i64,
    occurred_at: String,
    action: String,
    source: String,
    result: Option<String>,
    reason_code: Option<String>,
    reason_message: Option<String>,
    http_status: Option<i64>,
}

fn idle_refresh_status() -> UpstreamAccountRefreshStatus {
    UpstreamAccountRefreshStatus {
        state: UPSTREAM_ACCOUNT_REFRESH_STATE_IDLE.to_string(),
        last_attempt_at: None,
        last_success_at: None,
        next_attempt_at: None,
        source: None,
        error_code: None,
        error_message: None,
    }
}

fn refresh_status_from_row(row: UpstreamAccountRefreshStateRow) -> UpstreamAccountRefreshStatus {
    UpstreamAccountRefreshStatus {
        state: row.state,
        last_attempt_at: row.last_attempt_at,
        last_success_at: row.last_success_at,
        next_attempt_at: row.next_attempt_at,
        source: row.source,
        error_code: row.error_code,
        error_message: row.error_message,
    }
}

async fn load_refresh_state_row(
    pool: &Pool<Sqlite>,
    account_id: i64,
    operation: &str,
) -> Result<Option<UpstreamAccountRefreshStateRow>> {
    sqlx::query_as::<_, UpstreamAccountRefreshStateRow>(
        r#"
        SELECT
            account_id,
            operation,
            state,
            last_attempt_at,
            last_success_at,
            next_attempt_at,
            source,
            error_code,
            error_message
        FROM pool_upstream_account_refresh_state
        WHERE account_id = ?1 AND operation = ?2
        "#,
    )
    .bind(account_id)
    .bind(operation)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_upstream_account_refresh_status(
    pool: &Pool<Sqlite>,
    account_id: i64,
    operation: &str,
) -> Result<UpstreamAccountRefreshStatus> {
    Ok(load_refresh_state_row(pool, account_id, operation)
        .await?
        .map(refresh_status_from_row)
        .unwrap_or_else(idle_refresh_status))
}

pub(crate) async fn load_upstream_account_refresh_history(
    pool: &Pool<Sqlite>,
    account_id: i64,
    limit: i64,
) -> Result<Vec<UpstreamAccountRefreshRecord>> {
    let rows = sqlx::query_as::<_, UpstreamAccountRefreshHistoryRow>(
        r#"
        SELECT
            id,
            occurred_at,
            action,
            source,
            result,
            reason_code,
            reason_message,
            http_status
        FROM pool_upstream_account_events
        WHERE account_id = ?1
          AND action IN (?2, ?3, ?4)
        ORDER BY occurred_at DESC, id DESC
        LIMIT ?5
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_SUCCEEDED)
    .bind(UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_FAILED)
    .bind(UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REAUTH_REQUIRED)
    .bind(limit.max(0))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| UpstreamAccountRefreshRecord {
            id: row.id,
            occurred_at: row.occurred_at,
            action: row.action,
            source: row.source,
            result: row.result,
            reason_code: row.reason_code,
            reason_message: row.reason_message,
            http_status: row.http_status.and_then(|value| u16::try_from(value).ok()),
        })
        .collect())
}

pub(crate) async fn begin_upstream_account_refresh_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    operation: &str,
    source: &str,
) -> Result<()> {
    let now = format_utc_iso(Utc::now());
    info!(
        account_id,
        operation, source, "upstream account refresh attempt started"
    );
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_refresh_state (
            account_id, operation, state, last_attempt_at, source,
            error_code, error_message, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?4)
        ON CONFLICT(account_id, operation) DO UPDATE SET
            state = excluded.state,
            last_attempt_at = excluded.last_attempt_at,
            next_attempt_at = NULL,
            source = excluded.source,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(account_id)
    .bind(operation)
    .bind(UPSTREAM_ACCOUNT_REFRESH_STATE_REFRESHING)
    .bind(&now)
    .bind(source)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn complete_upstream_account_refresh_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    operation: &str,
    source: &str,
    next_attempt_at: Option<&str>,
) -> Result<()> {
    let now = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_refresh_state (
            account_id, operation, state, last_attempt_at, last_success_at,
            next_attempt_at, source, error_code, error_message, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, NULL, NULL, ?4)
        ON CONFLICT(account_id, operation) DO UPDATE SET
            state = excluded.state,
            last_success_at = excluded.last_success_at,
            next_attempt_at = excluded.next_attempt_at,
            source = excluded.source,
            error_code = NULL,
            error_message = NULL,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(account_id)
    .bind(operation)
    .bind(UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED)
    .bind(&now)
    .bind(next_attempt_at)
    .bind(source)
    .execute(pool)
    .await?;

    let (action, reason_code) = match operation {
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS => (
            UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_SUCCEEDED,
            "credential_refresh_ok",
        ),
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS => (
            UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_SUCCEEDED,
            "usage_windows_refresh_ok",
        ),
        _ => return Ok(()),
    };
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action,
            source,
            reason_code: Some(reason_code),
            reason_message: None,
            http_status: None,
            failure_kind: None,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now,
        },
    )
    .await
}

pub(crate) async fn defer_upstream_account_refresh_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    operation: &str,
    source: &str,
    retry_after_secs: u64,
    error_message: &str,
) -> Result<()> {
    let now = format_utc_iso(Utc::now());
    let next_attempt_at = format_utc_iso(
        Utc::now()
            + ChronoDuration::seconds(
                retry_after_secs.max(UPSTREAM_ACCOUNT_REFRESH_RETRY_INTERVAL_SECS as u64) as i64,
            ),
    );
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_refresh_state (
            account_id, operation, state, last_attempt_at, next_attempt_at,
            source, error_code, error_message, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?4)
        ON CONFLICT(account_id, operation) DO UPDATE SET
            state = excluded.state,
            last_attempt_at = excluded.last_attempt_at,
            next_attempt_at = excluded.next_attempt_at,
            source = excluded.source,
            error_code = excluded.error_code,
            error_message = excluded.error_message,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(account_id)
    .bind(operation)
    .bind(UPSTREAM_ACCOUNT_REFRESH_STATE_RETRYING)
    .bind(&now)
    .bind(next_attempt_at)
    .bind(source)
    .bind("egress_throttled")
    .bind(error_message)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_upstream_account_refresh_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    operation: &str,
    source: &str,
    error: &str,
    error_code: &str,
    error_message: &str,
    partial: bool,
    http_status: Option<StatusCode>,
) -> Result<()> {
    let previous = load_refresh_state_row(pool, account_id, operation).await?;
    let is_reauth = operation == UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS
        && (matches!(
            error_code,
            "refresh_token_invalid" | "refresh_token_missing" | "reauth_required"
        ) || is_reauth_error(&anyhow!("{error}")));
    let state = if is_reauth {
        UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH
    } else if partial {
        UPSTREAM_ACCOUNT_REFRESH_STATE_PARTIAL
    } else {
        UPSTREAM_ACCOUNT_REFRESH_STATE_RETRYING
    };
    let next_attempt_at = if is_reauth {
        None
    } else {
        Some(format_utc_iso(
            Utc::now() + ChronoDuration::seconds(UPSTREAM_ACCOUNT_REFRESH_RETRY_INTERVAL_SECS),
        ))
    };
    let now = format_utc_iso(Utc::now());
    warn!(
        account_id,
        operation, source, error_code, partial, "upstream account refresh attempt failed"
    );
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_refresh_state (
            account_id, operation, state, last_attempt_at, last_success_at,
            next_attempt_at, source, error_code, error_message, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?4
        )
        ON CONFLICT(account_id, operation) DO UPDATE SET
            state = excluded.state,
            last_attempt_at = excluded.last_attempt_at,
            last_success_at = pool_upstream_account_refresh_state.last_success_at,
            next_attempt_at = excluded.next_attempt_at,
            source = excluded.source,
            error_code = excluded.error_code,
            error_message = excluded.error_message,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(account_id)
    .bind(operation)
    .bind(state)
    .bind(&now)
    .bind(next_attempt_at.as_deref())
    .bind(source)
    .bind(error_code)
    .bind(error_message)
    .execute(pool)
    .await?;

    let should_record_event = previous.as_ref().is_none_or(|previous| {
        previous.state == UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED
            || previous.state == UPSTREAM_ACCOUNT_REFRESH_STATE_IDLE
            || previous.error_code.as_deref() != Some(error_code)
            || previous.error_message.as_deref() != Some(error_message)
    });
    if !should_record_event {
        return Ok(());
    }

    let (action, reason_code) = match operation {
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS if is_reauth => (
            UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REAUTH_REQUIRED,
            if error_code == "refresh_token_missing" {
                "refresh_token_missing"
            } else {
                "refresh_token_invalid"
            },
        ),
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS => (
            UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_FAILED,
            error_code,
        ),
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS if partial => (
            UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_PARTIAL,
            error_code,
        ),
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS => (
            UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_FAILED,
            error_code,
        ),
        _ => return Ok(()),
    };
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status,
            failure_kind: None,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now,
        },
    )
    .await
}

pub(crate) fn classify_refresh_error(error: &str, operation: &str) -> (String, String) {
    let lower = error.to_ascii_lowercase();
    if lower.contains("invalid_grant")
        || (operation == UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS
            && (lower.contains("revoked")
                || lower.contains("expired")
                || lower.contains("refresh token") && lower.contains("invalid")))
    {
        return (
            "refresh_token_invalid".to_string(),
            "Refresh Token 已失效，需要重新授权。".to_string(),
        );
    }
    if lower.contains("401") {
        return (
            "upstream_http_401".to_string(),
            "上游返回 HTTP 401，等待下一分钟重试。".to_string(),
        );
    }
    if lower.contains("403") {
        return (
            "upstream_http_403".to_string(),
            "上游返回 HTTP 403，等待下一分钟重试。".to_string(),
        );
    }
    if let Some(status) = refresh_http_status(error).filter(StatusCode::is_server_error) {
        return (
            "upstream_http_5xx".to_string(),
            format!("上游返回 {}: 等待下一分钟重试。", status.as_u16()),
        );
    }
    if lower.contains("decode") || lower.contains("parse") || lower.contains("missing") {
        return (
            format!("{operation}_response_parse_failed"),
            "上游响应解析失败，等待下一分钟重试。".to_string(),
        );
    }
    if lower.contains("transport")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connect")
    {
        return (
            "transport_failure".to_string(),
            "上游请求暂时失败，等待下一分钟重试。".to_string(),
        );
    }
    (
        format!("{operation}_refresh_failed"),
        "刷新阶段失败，等待下一分钟重试。".to_string(),
    )
}
