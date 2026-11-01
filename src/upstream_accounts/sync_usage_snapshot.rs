use super::*;

pub(crate) const OAUTH_USAGE_SNAPSHOT_CONFLICT_MESSAGE: &str =
    "usage snapshot changed while OAuth credentials were being probed";

#[derive(Debug, Clone)]
pub(crate) struct UsageSnapshotVersion {
    pub(crate) captured_at: String,
    pub(crate) id: i64,
}

pub(crate) fn usage_refresh_failure_message(error_code: &str, error_message: &str) -> String {
    if error_code == "upstream_http_429_quota_exhausted" {
        format!("quota exhausted: {error_message}")
    } else {
        error_message.to_string()
    }
}

pub(crate) fn imported_oauth_probe_error_tuple(error: anyhow::Error) -> (StatusCode, String) {
    if error
        .chain()
        .any(|cause| cause.to_string() == OAUTH_USAGE_SNAPSHOT_CONFLICT_MESSAGE)
    {
        (
            StatusCode::CONFLICT,
            "usage data changed while OAuth credentials were being probed; retry the operation"
                .to_string(),
        )
    } else {
        internal_error_tuple(error)
    }
}

pub(crate) async fn persist_usage_snapshot_with_versions(
    pool: &Pool<Sqlite>,
    account_id: i64,
    effective_plan_type: Option<&str>,
    snapshot: &NormalizedUsageSnapshot,
    retention_days: u64,
    expected_lease_token: Option<&str>,
    expected_usage_version: Option<Option<UsageSnapshotVersion>>,
) -> Result<bool> {
    let now_iso = format_utc_iso(Utc::now());
    let latest =
        super::sync_account_imports_tags::load_latest_usage_sample(pool, account_id).await?;
    let primary = snapshot.primary.clone().or_else(|| {
        latest.as_ref().and_then(|sample| {
            Some(NormalizedUsageWindow {
                used_percent: sample.primary_used_percent?,
                window_duration_mins: sample.primary_window_minutes?,
                resets_at: sample.primary_resets_at.clone(),
            })
        })
    });
    let secondary = snapshot.secondary.clone().or_else(|| {
        latest.as_ref().and_then(|sample| {
            Some(NormalizedUsageWindow {
                used_percent: sample.secondary_used_percent?,
                window_duration_mins: sample.secondary_window_minutes?,
                resets_at: sample.secondary_resets_at.clone(),
            })
        })
    });
    let credits = snapshot.credits.clone().or_else(|| {
        latest.as_ref().and_then(|sample| {
            sample
                .credits_has_credits
                .map(|has_credits| CreditsSnapshot {
                    has_credits: has_credits != 0,
                    unlimited: sample.credits_unlimited.unwrap_or_default() != 0,
                    balance: sample.credits_balance.clone(),
                })
        })
    });
    let insert_result = sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        )
        SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
        WHERE (
            ?15 IS NULL OR EXISTS (
                SELECT 1
                FROM pool_upstream_account_refresh_state
                WHERE account_id = ?1
                  AND operation = ?16
                  AND lease_token = ?15
            )
        )
          AND (
              ?17 = 0
              OR (?18 = 0 AND NOT EXISTS (
                  SELECT 1
                  FROM pool_upstream_account_limit_samples
                  WHERE account_id = ?1
              ))
              OR (?18 = 1 AND (?19, ?20) = (
                  SELECT captured_at, id
                  FROM pool_upstream_account_limit_samples
                  WHERE account_id = ?1
                  ORDER BY captured_at DESC, id DESC
                  LIMIT 1
              ))
          )
        "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .bind(&snapshot.limit_id)
    .bind(&snapshot.limit_name)
    .bind(
        snapshot
            .plan_type
            .clone()
            .or_else(|| effective_plan_type.map(str::to_string)),
    )
    .bind(primary.as_ref().map(|value| value.used_percent))
    .bind(primary.as_ref().map(|value| value.window_duration_mins))
    .bind(primary.as_ref().and_then(|value| value.resets_at.clone()))
    .bind(secondary.as_ref().map(|value| value.used_percent))
    .bind(secondary.as_ref().map(|value| value.window_duration_mins))
    .bind(secondary.as_ref().and_then(|value| value.resets_at.clone()))
    .bind(
        credits
            .as_ref()
            .map(|value| if value.has_credits { 1 } else { 0 }),
    )
    .bind(
        credits
            .as_ref()
            .map(|value| if value.unlimited { 1 } else { 0 }),
    )
    .bind(credits.as_ref().and_then(|value| value.balance.clone()))
    .bind(expected_lease_token)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS)
    .bind(i64::from(expected_usage_version.is_some()))
    .bind(i64::from(
        expected_usage_version
            .as_ref()
            .and_then(Option::as_ref)
            .is_some(),
    ))
    .bind(
        expected_usage_version
            .as_ref()
            .and_then(Option::as_ref)
            .map(|value| value.captured_at.as_str()),
    )
    .bind(
        expected_usage_version
            .as_ref()
            .and_then(Option::as_ref)
            .map(|value| value.id),
    )
    .execute(pool)
    .await?;
    if (expected_lease_token.is_some() || expected_usage_version.is_some())
        && insert_result.rows_affected() == 0
    {
        return Ok(false);
    }

    let retention_cutoff = format_utc_iso(Utc::now() - ChronoDuration::days(retention_days as i64));
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1
          AND captured_at < ?2
          AND (
              ?3 IS NULL OR EXISTS (
                  SELECT 1
                  FROM pool_upstream_account_refresh_state
                  WHERE account_id = ?1
                    AND operation = ?4
                    AND lease_token = ?3
              )
          )
        "#,
    )
    .bind(account_id)
    .bind(retention_cutoff)
    .bind(expected_lease_token)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS)
    .execute(pool)
    .await?;
    Ok(true)
}
