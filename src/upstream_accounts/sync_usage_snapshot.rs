use super::*;

pub(crate) const OAUTH_USAGE_SNAPSHOT_CONFLICT_MESSAGE: &str =
    "usage snapshot changed while OAuth credentials were being probed";

#[derive(Debug, Clone, PartialEq, Eq)]
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

fn normalized_window_from_sample(
    used_percent: Option<f64>,
    window_duration_mins: Option<i64>,
    resets_at: Option<String>,
) -> Option<NormalizedUsageWindow> {
    Some(NormalizedUsageWindow {
        used_percent: used_percent?,
        window_duration_mins: window_duration_mins?,
        resets_at,
    })
}

pub(crate) async fn persist_unknown_reserve_usage_snapshot_with_lease(
    pool: &Pool<Sqlite>,
    account_id: i64,
    effective_plan_type: Option<&str>,
    retention_days: u64,
    expected_lease_token: Option<&str>,
) -> Result<bool> {
    let latest =
        super::sync_account_imports_tags::load_latest_usage_sample(pool, account_id).await?;
    if let Some(latest) = latest.as_ref() {
        let captured_at = format_utc_iso(Utc::now());
        let update_result = sqlx::query(
            r#"
            UPDATE pool_upstream_account_limit_samples
            SET reserve_status = ?1,
                reserve_allowed = NULL,
                reserve_limit_reached = NULL,
                reserve_primary_used_percent = NULL,
                reserve_primary_window_minutes = NULL,
                reserve_primary_resets_at = NULL,
                reserve_primary_available_percent = NULL,
                reserve_secondary_used_percent = NULL,
                reserve_secondary_window_minutes = NULL,
                reserve_secondary_resets_at = NULL,
                reserve_secondary_available_percent = NULL,
                reserve_normal_model_slug = NULL,
                reserve_metered_feature = NULL,
                reserve_binding_source = NULL,
                reserve_captured_at = ?2
            WHERE id = ?3
              AND captured_at = ?4
              AND (
                  ?5 IS NULL OR EXISTS (
                      SELECT 1
                      FROM pool_upstream_account_refresh_state
                      WHERE account_id = ?6
                        AND operation = ?7
                        AND lease_token = ?5
                  )
              )
            "#,
        )
        .bind(RESERVE_AVAILABILITY_UNKNOWN)
        .bind(&captured_at)
        .bind(latest.id)
        .bind(&latest.captured_at)
        .bind(expected_lease_token)
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS)
        .execute(pool)
        .await?;
        if expected_lease_token.is_some() && update_result.rows_affected() == 0 {
            return Ok(false);
        }
        return Ok(update_result.rows_affected() > 0);
    }
    let snapshot = NormalizedUsageSnapshot {
        plan_type: latest.as_ref().and_then(|sample| sample.plan_type.clone()),
        limit_id: latest
            .as_ref()
            .and_then(|sample| sample.limit_id.clone())
            .unwrap_or_else(|| DEFAULT_USAGE_LIMIT_ID.to_string()),
        limit_name: latest.as_ref().and_then(|sample| sample.limit_name.clone()),
        primary: latest.as_ref().and_then(|sample| {
            normalized_window_from_sample(
                sample.primary_used_percent,
                sample.primary_window_minutes,
                sample.primary_resets_at.clone(),
            )
        }),
        secondary: latest.as_ref().and_then(|sample| {
            normalized_window_from_sample(
                sample.secondary_used_percent,
                sample.secondary_window_minutes,
                sample.secondary_resets_at.clone(),
            )
        }),
        credits: latest.as_ref().and_then(|sample| {
            sample
                .credits_has_credits
                .map(|has_credits| CreditsSnapshot {
                    has_credits: has_credits != 0,
                    unlimited: sample.credits_unlimited.unwrap_or_default() != 0,
                    balance: sample.credits_balance.clone(),
                })
        }),
        reserve: Some(NormalizedReserveQuotaSnapshot {
            availability: RESERVE_AVAILABILITY_UNKNOWN.to_string(),
            allowed: None,
            limit_reached: None,
            normal_model_slug: None,
            metered_feature: None,
            binding_source: None,
            primary: None,
            secondary: None,
            primary_available_percent: None,
            secondary_available_percent: None,
        }),
    };
    persist_usage_snapshot_with_versions(
        pool,
        account_id,
        effective_plan_type,
        &snapshot,
        retention_days,
        expected_lease_token,
        None,
    )
    .await
}

pub(crate) fn probe_error_tuple(error: anyhow::Error) -> (StatusCode, String) {
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

pub(crate) async fn ensure_usage(
    connection: &mut sqlx::SqliteConnection,
    account_id: i64,
    expected: Option<&Option<UsageSnapshotVersion>>,
) -> std::result::Result<(), (StatusCode, String)> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let current = sqlx::query_as::<_, (String, i64)>(
        "SELECT captured_at, id FROM pool_upstream_account_limit_samples WHERE account_id = ?1 ORDER BY captured_at DESC, id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(internal_error_tuple)?
    .map(|(captured_at, id)| UsageSnapshotVersion { captured_at, id });
    if current != expected.clone() {
        return Err((
            StatusCode::CONFLICT,
            "usage data changed while OAuth credentials were being probed; retry the operation"
                .to_string(),
        ));
    }
    Ok(())
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
    let reserve =
        snapshot
            .reserve
            .as_ref()
            .cloned()
            .unwrap_or_else(|| NormalizedReserveQuotaSnapshot {
                availability: RESERVE_AVAILABILITY_UNKNOWN.to_string(),
                allowed: None,
                limit_reached: None,
                normal_model_slug: None,
                metered_feature: None,
                binding_source: None,
                primary: None,
                secondary: None,
                primary_available_percent: None,
                secondary_available_percent: None,
            });
    let insert_result = sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance,
            reserve_status,
            reserve_allowed, reserve_limit_reached,
            reserve_primary_used_percent, reserve_primary_window_minutes, reserve_primary_resets_at,
            reserve_primary_available_percent,
            reserve_secondary_used_percent, reserve_secondary_window_minutes, reserve_secondary_resets_at,
            reserve_secondary_available_percent,
            reserve_captured_at,
            reserve_normal_model_slug, reserve_metered_feature, reserve_binding_source
        )
        SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
               ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26,
               ?27, ?28, ?29
        WHERE (
            ?30 IS NULL OR EXISTS (
                SELECT 1
                FROM pool_upstream_account_refresh_state
                WHERE account_id = ?1
                  AND operation = ?31
                  AND lease_token = ?30
            )
        )
          AND (
              ?32 = 0
              OR (?33 = 0 AND NOT EXISTS (
                  SELECT 1
                  FROM pool_upstream_account_limit_samples
                  WHERE account_id = ?1
              ))
              OR (?33 = 1 AND (?34, ?35) = (
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
    .bind(&reserve.availability)
    .bind(reserve.allowed.map(i64::from))
    .bind(reserve.limit_reached.map(i64::from))
    .bind(reserve.primary.as_ref().map(|value| value.used_percent))
    .bind(reserve.primary.as_ref().map(|value| value.window_duration_mins))
    .bind(reserve.primary.as_ref().and_then(|value| value.resets_at.clone()))
    .bind(reserve.primary_available_percent)
    .bind(reserve.secondary.as_ref().map(|value| value.used_percent))
    .bind(reserve.secondary.as_ref().map(|value| value.window_duration_mins))
    .bind(reserve.secondary.as_ref().and_then(|value| value.resets_at.clone()))
    .bind(reserve.secondary_available_percent)
    .bind(&now_iso)
    .bind(&reserve.normal_model_slug)
    .bind(&reserve.metered_feature)
    .bind(&reserve.binding_source)
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
