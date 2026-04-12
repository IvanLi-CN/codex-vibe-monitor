use super::*;

pub(crate) async fn run_upstream_account_maintenance_once(state: Arc<AppState>) -> Result<()> {
    expire_pending_login_sessions(&state.pool).await?;
    cleanup_expired_oauth_mailbox_sessions(state.as_ref()).await?;
    let Some(_) = state.upstream_accounts.crypto_key else {
        return Ok(());
    };
    let routing = load_pool_routing_settings(&state.pool).await?;
    let maintenance = resolve_pool_routing_maintenance_settings(&routing, &state.config);
    let candidates = load_maintenance_candidates(&state.pool).await?;
    let dispatch_plans = resolve_due_maintenance_dispatch_plans(
        candidates,
        maintenance,
        state.config.upstream_accounts_refresh_lead_time,
        Utc::now(),
    );

    let mut queued = 0usize;
    let mut deduped = 0usize;
    let mut failed = 0usize;
    let mut high_frequency_due = 0usize;
    let mut priority_due = 0usize;
    let mut secondary_due = 0usize;
    for plan in dispatch_plans {
        match plan.tier {
            MaintenanceTier::HighFrequency => high_frequency_due += 1,
            MaintenanceTier::Priority => priority_due += 1,
            MaintenanceTier::Secondary => secondary_due += 1,
        }
        match state
            .upstream_accounts
            .account_ops
            .dispatch_maintenance_sync(state.clone(), plan)
        {
            Ok(MaintenanceQueueOutcome::Queued) => queued += 1,
            Ok(MaintenanceQueueOutcome::Deduped) => deduped += 1,
            Err(err) => {
                failed += 1;
                warn!(
                    account_id = plan.account_id,
                    tier = ?plan.tier,
                    error = %err,
                    "failed to dispatch upstream OAuth maintenance"
                );
            }
        }
    }

    info!(
        candidates = queued + deduped + failed,
        high_frequency_due,
        priority_due,
        secondary_due,
        queued,
        deduped,
        failed,
        "upstream account maintenance pass finished"
    );

    Ok(())
}

pub(crate) async fn ensure_integer_column_with_default(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
    default_value: &str,
) -> Result<()> {
    let pragma_statement = format!("PRAGMA table_info({table_name})");
    let columns: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&pragma_statement).fetch_all(pool).await?;
    if columns
        .iter()
        .any(|(_, name, _, _, _, _)| name == column_name)
    {
        return Ok(());
    }

    let statement = format!(
        "ALTER TABLE {table_name} ADD COLUMN {column_name} INTEGER NOT NULL DEFAULT {default_value}"
    );
    sqlx::query(&statement).execute(pool).await?;

    Ok(())
}

pub(crate) async fn load_maintenance_candidates(
    pool: &Pool<Sqlite>,
) -> Result<Vec<MaintenanceCandidateRow>> {
    sqlx::query_as::<_, MaintenanceCandidateRow>(
        r#"
        SELECT
            account.id,
            account.status,
            account.last_synced_at,
            account.last_action_source,
            account.last_action_at,
            account.last_selected_at,
            account.last_error_at,
            account.last_error,
            account.last_route_failure_at,
            account.last_route_failure_kind,
            account.last_action_reason_code,
            account.cooldown_until,
            account.temporary_route_failure_streak_started_at,
            account.token_expires_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.primary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_resets_at,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.secondary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_resets_at,
            (
                SELECT sample.credits_has_credits
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_has_credits,
            (
                SELECT sample.credits_unlimited
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_unlimited,
            (
                SELECT sample.credits_balance
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_balance
        FROM pool_upstream_accounts account
        WHERE account.kind = ?1
          AND account.enabled = 1
          AND account.status <> ?2
        ORDER BY account.id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_maintenance_candidate(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<MaintenanceCandidateRow>> {
    sqlx::query_as::<_, MaintenanceCandidateRow>(
        r#"
        SELECT
            account.id,
            account.status,
            account.last_synced_at,
            account.last_action_source,
            account.last_action_at,
            account.last_selected_at,
            account.last_error_at,
            account.last_error,
            account.last_route_failure_at,
            account.last_route_failure_kind,
            account.last_action_reason_code,
            account.cooldown_until,
            account.temporary_route_failure_streak_started_at,
            account.token_expires_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.primary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_resets_at,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.secondary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_resets_at,
            (
                SELECT sample.credits_has_credits
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_has_credits,
            (
                SELECT sample.credits_unlimited
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_unlimited,
            (
                SELECT sample.credits_balance
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_balance
        FROM pool_upstream_accounts account
        WHERE account.id = ?1
          AND account.kind = ?2
          AND account.enabled = 1
          AND account.status <> ?3
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) fn maintenance_refresh_due(
    candidate: &MaintenanceCandidateRow,
    refresh_lead_time: Duration,
    now: DateTime<Utc>,
) -> bool {
    candidate
        .token_expires_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .map(|expires| expires <= now + ChronoDuration::seconds(refresh_lead_time.as_secs() as i64))
        .unwrap_or(true)
}

pub(crate) fn maintenance_candidate_has_complete_usage(
    candidate: &MaintenanceCandidateRow,
) -> bool {
    candidate.primary_used_percent.is_some() && candidate.secondary_used_percent.is_some()
}

pub(crate) fn maintenance_candidate_snapshot_exhausted(
    candidate: &MaintenanceCandidateRow,
) -> bool {
    persisted_usage_snapshot_is_exhausted(
        candidate.primary_used_percent,
        candidate.secondary_used_percent,
        candidate.credits_has_credits.map(|value| value != 0),
        candidate.credits_unlimited.map(|value| value != 0),
        candidate.credits_balance.as_deref(),
    )
}

pub(crate) fn maintenance_candidate_health_status(
    candidate: &MaintenanceCandidateRow,
) -> &'static str {
    derive_upstream_account_health_status(
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        true,
        &candidate.status,
        candidate.last_error.as_deref(),
        candidate.last_error_at.as_deref(),
        candidate.last_route_failure_at.as_deref(),
        candidate.last_route_failure_kind.as_deref(),
        candidate.last_action_reason_code.as_deref(),
    )
}

pub(crate) fn maintenance_candidate_work_status(
    candidate: &MaintenanceCandidateRow,
    now: DateTime<Utc>,
) -> &'static str {
    let health_status = maintenance_candidate_health_status(candidate);
    let sync_state = derive_upstream_account_sync_state(true, &candidate.status);
    derive_upstream_account_work_status(
        true,
        &candidate.status,
        health_status,
        sync_state,
        maintenance_candidate_snapshot_exhausted(candidate),
        candidate.cooldown_until.as_deref(),
        candidate.last_error_at.as_deref(),
        candidate.last_route_failure_at.as_deref(),
        candidate.last_route_failure_kind.as_deref(),
        candidate.last_action_reason_code.as_deref(),
        candidate
            .temporary_route_failure_streak_started_at
            .as_deref(),
        candidate.last_selected_at.as_deref(),
        now,
    )
}

pub(crate) fn maintenance_candidate_is_high_frequency(
    candidate: &MaintenanceCandidateRow,
    now: DateTime<Utc>,
) -> bool {
    matches!(
        maintenance_candidate_work_status(candidate, now),
        UPSTREAM_ACCOUNT_WORK_STATUS_WORKING | UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
    )
}

pub(crate) fn maintenance_candidate_is_available(candidate: &MaintenanceCandidateRow) -> bool {
    maintenance_candidate_has_complete_usage(candidate)
        && !maintenance_candidate_snapshot_exhausted(candidate)
}

pub(crate) fn maintenance_candidate_force_priority(
    candidate: &MaintenanceCandidateRow,
    refresh_lead_time: Duration,
    now: DateTime<Utc>,
) -> bool {
    candidate.status == UPSTREAM_ACCOUNT_STATUS_ERROR
        || maintenance_refresh_due(candidate, refresh_lead_time, now)
        || !maintenance_candidate_has_complete_usage(candidate)
}

pub(crate) fn compare_maintenance_candidates(
    lhs: &MaintenanceCandidateRow,
    rhs: &MaintenanceCandidateRow,
) -> std::cmp::Ordering {
    lhs.secondary_used_percent
        .unwrap_or(100.0)
        .partial_cmp(&rhs.secondary_used_percent.unwrap_or(100.0))
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            lhs.primary_used_percent
                .unwrap_or(100.0)
                .partial_cmp(&rhs.primary_used_percent.unwrap_or(100.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            lhs.last_synced_at
                .as_deref()
                .cmp(&rhs.last_synced_at.as_deref())
        })
        .then_with(|| lhs.id.cmp(&rhs.id))
}

pub(crate) fn maintenance_last_sync_attempt_at(
    candidate: &MaintenanceCandidateRow,
) -> Option<DateTime<Utc>> {
    let last_synced_at = candidate
        .last_synced_at
        .as_deref()
        .and_then(parse_rfc3339_utc);
    let last_sync_action_at = candidate
        .last_action_source
        .as_deref()
        .filter(|source| {
            matches!(
                *source,
                UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE
                    | UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL
                    | UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_POST_CREATE
            )
        })
        .and(candidate.last_action_at.as_deref())
        .as_deref()
        .and_then(parse_rfc3339_utc);

    [last_synced_at, last_sync_action_at]
        .into_iter()
        .flatten()
        .max()
}

pub(crate) fn maintenance_last_interval_anchor_at(
    candidate: &MaintenanceCandidateRow,
) -> Option<DateTime<Utc>> {
    let last_sync_attempt_at = maintenance_last_sync_attempt_at(candidate);
    let last_error_at = (candidate.status == UPSTREAM_ACCOUNT_STATUS_ERROR)
        .then(|| {
            candidate
                .last_error_at
                .as_deref()
                .and_then(parse_rfc3339_utc)
        })
        .flatten();

    [last_sync_attempt_at, last_error_at]
        .into_iter()
        .flatten()
        .max()
}

pub(crate) fn maintenance_last_attempt_recorded_after_reset(
    candidate: &MaintenanceCandidateRow,
    reset_at: DateTime<Utc>,
) -> bool {
    maintenance_last_sync_attempt_at(candidate)
        .is_some_and(|last_attempt_at| last_attempt_at >= reset_at)
}

pub(crate) fn maintenance_interval_is_due(
    candidate: &MaintenanceCandidateRow,
    interval_secs: u64,
    now: DateTime<Utc>,
) -> bool {
    maintenance_last_interval_anchor_at(candidate)
        .map(|last| now.signed_duration_since(last).num_seconds() >= interval_secs as i64)
        .unwrap_or(true)
}

pub(crate) fn maintenance_interval_for_tier(
    tier: MaintenanceTier,
    settings: PoolRoutingMaintenanceSettings,
) -> u64 {
    match tier {
        MaintenanceTier::HighFrequency => MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
        MaintenanceTier::Priority => settings.primary_sync_interval_secs,
        MaintenanceTier::Secondary => settings.secondary_sync_interval_secs,
    }
}

pub(crate) fn maintenance_window_reset_due(
    candidate: &MaintenanceCandidateRow,
    resets_at: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    let Some(reset_at) = resets_at.and_then(parse_rfc3339_utc) else {
        return false;
    };
    reset_at <= now && !maintenance_last_attempt_recorded_after_reset(candidate, reset_at)
}

pub(crate) fn maintenance_reset_due(
    candidate: &MaintenanceCandidateRow,
    now: DateTime<Utc>,
) -> bool {
    maintenance_window_reset_due(candidate, candidate.primary_resets_at.as_deref(), now)
        || maintenance_window_reset_due(candidate, candidate.secondary_resets_at.as_deref(), now)
}

pub(crate) fn maintenance_plan_is_due(
    candidate: &MaintenanceCandidateRow,
    tier: MaintenanceTier,
    settings: PoolRoutingMaintenanceSettings,
    now: DateTime<Utc>,
) -> bool {
    maintenance_reset_due(candidate, now)
        || maintenance_interval_is_due(
            candidate,
            maintenance_interval_for_tier(tier, settings),
            now,
        )
}

pub(crate) async fn load_maintenance_candidates_ranked_before(
    pool: &Pool<Sqlite>,
    candidate: &MaintenanceCandidateRow,
    offset: usize,
    limit: usize,
) -> Result<Vec<MaintenanceCandidateRow>> {
    let secondary_used_percent = candidate.secondary_used_percent.unwrap_or(100.0);
    let primary_used_percent = candidate.primary_used_percent.unwrap_or(100.0);
    let last_synced_sort_key = candidate.last_synced_at.as_deref().unwrap_or("");
    sqlx::query_as::<_, MaintenanceCandidateRow>(
        r#"
        WITH ranked_candidates AS (
            SELECT
                account.id,
                account.status,
                account.last_synced_at,
                account.last_action_source,
                account.last_action_at,
                account.last_selected_at,
                account.last_error_at,
                account.last_error,
                account.last_route_failure_at,
                account.last_route_failure_kind,
                account.last_action_reason_code,
                account.cooldown_until,
                account.temporary_route_failure_streak_started_at,
                account.token_expires_at,
                (
                    SELECT sample.primary_used_percent
                    FROM pool_upstream_account_limit_samples sample
                    WHERE sample.account_id = account.id
                    ORDER BY sample.captured_at DESC
                    LIMIT 1
                ) AS primary_used_percent,
                (
                    SELECT sample.primary_resets_at
                    FROM pool_upstream_account_limit_samples sample
                    WHERE sample.account_id = account.id
                    ORDER BY sample.captured_at DESC
                    LIMIT 1
                ) AS primary_resets_at,
                (
                    SELECT sample.secondary_used_percent
                    FROM pool_upstream_account_limit_samples sample
                    WHERE sample.account_id = account.id
                    ORDER BY sample.captured_at DESC
                    LIMIT 1
                ) AS secondary_used_percent,
                (
                    SELECT sample.secondary_resets_at
                    FROM pool_upstream_account_limit_samples sample
                    WHERE sample.account_id = account.id
                    ORDER BY sample.captured_at DESC
                    LIMIT 1
                ) AS secondary_resets_at,
                (
                    SELECT sample.credits_has_credits
                    FROM pool_upstream_account_limit_samples sample
                    WHERE sample.account_id = account.id
                    ORDER BY sample.captured_at DESC
                    LIMIT 1
                ) AS credits_has_credits,
                (
                    SELECT sample.credits_unlimited
                    FROM pool_upstream_account_limit_samples sample
                    WHERE sample.account_id = account.id
                    ORDER BY sample.captured_at DESC
                    LIMIT 1
                ) AS credits_unlimited,
                (
                    SELECT sample.credits_balance
                    FROM pool_upstream_account_limit_samples sample
                    WHERE sample.account_id = account.id
                    ORDER BY sample.captured_at DESC
                    LIMIT 1
                ) AS credits_balance
            FROM pool_upstream_accounts account
            WHERE account.kind = ?1
              AND account.enabled = 1
              AND account.status <> ?2
        )
        SELECT *
        FROM ranked_candidates
        WHERE
            COALESCE(secondary_used_percent, 100.0) < ?3
            OR (
                COALESCE(secondary_used_percent, 100.0) = ?3
                AND COALESCE(primary_used_percent, 100.0) < ?4
            )
            OR (
                COALESCE(secondary_used_percent, 100.0) = ?3
                AND COALESCE(primary_used_percent, 100.0) = ?4
                AND COALESCE(last_synced_at, '') < ?5
            )
            OR (
                COALESCE(secondary_used_percent, 100.0) = ?3
                AND COALESCE(primary_used_percent, 100.0) = ?4
                AND COALESCE(last_synced_at, '') = ?5
                AND id < ?6
            )
        ORDER BY
            COALESCE(secondary_used_percent, 100.0) ASC,
            COALESCE(primary_used_percent, 100.0) ASC,
            COALESCE(last_synced_at, '') ASC,
            id ASC
        LIMIT ?7 OFFSET ?8
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
    .bind(secondary_used_percent)
    .bind(primary_used_percent)
    .bind(last_synced_sort_key)
    .bind(candidate.id)
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn current_maintenance_interval_for_queued_high_frequency_candidate(
    state: &AppState,
    candidate: &MaintenanceCandidateRow,
    now: DateTime<Utc>,
) -> Result<u64> {
    let routing = load_pool_routing_settings(&state.pool).await?;
    let settings = resolve_pool_routing_maintenance_settings(&routing, &state.config);
    if maintenance_candidate_force_priority(
        candidate,
        state.config.upstream_accounts_refresh_lead_time,
        now,
    ) {
        return Ok(settings.primary_sync_interval_secs);
    }
    if !maintenance_candidate_is_available(candidate) {
        return Ok(settings.secondary_sync_interval_secs);
    }

    let cap = settings.priority_available_account_cap.max(1);
    let mut better_available = 0usize;
    let mut offset = 0usize;
    loop {
        let batch =
            load_maintenance_candidates_ranked_before(&state.pool, candidate, offset, cap).await?;
        if batch.is_empty() {
            return Ok(settings.primary_sync_interval_secs);
        }
        for other in &batch {
            if maintenance_candidate_is_high_frequency(other, now)
                || maintenance_candidate_force_priority(
                    other,
                    state.config.upstream_accounts_refresh_lead_time,
                    now,
                )
                || !maintenance_candidate_is_available(other)
            {
                continue;
            }
            better_available += 1;
            if better_available >= settings.priority_available_account_cap {
                return Ok(settings.secondary_sync_interval_secs);
            }
        }
        if batch.len() < cap {
            return Ok(settings.primary_sync_interval_secs);
        }
        offset += batch.len();
    }
}

pub(crate) async fn execute_queued_maintenance_sync(
    state: &AppState,
    plan: MaintenanceDispatchPlan,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    let now = Utc::now();
    let Some(candidate) = load_maintenance_candidate(&state.pool, id).await? else {
        return Ok(None);
    };
    let interval_secs = if matches!(plan.tier, MaintenanceTier::HighFrequency)
        && !maintenance_candidate_is_high_frequency(&candidate, now)
    {
        current_maintenance_interval_for_queued_high_frequency_candidate(state, &candidate, now)
            .await?
    } else {
        plan.sync_interval_secs
    };
    if !maintenance_reset_due(&candidate, now)
        && !maintenance_interval_is_due(&candidate, interval_secs, now)
    {
        return Ok(None);
    }

    sync_upstream_account_by_id(state, id, SyncCause::Maintenance).await
}

pub(crate) fn resolve_due_maintenance_dispatch_plans(
    candidates: Vec<MaintenanceCandidateRow>,
    settings: PoolRoutingMaintenanceSettings,
    refresh_lead_time: Duration,
    now: DateTime<Utc>,
) -> Vec<MaintenanceDispatchPlan> {
    let mut forced_priority = Vec::new();
    let mut high_frequency = Vec::new();
    let mut ranked_available = Vec::new();
    let mut secondary = Vec::new();

    for candidate in candidates {
        if maintenance_candidate_is_high_frequency(&candidate, now) {
            high_frequency.push(candidate);
        } else if maintenance_candidate_force_priority(&candidate, refresh_lead_time, now) {
            forced_priority.push(candidate);
        } else if maintenance_candidate_is_available(&candidate) {
            ranked_available.push(candidate);
        } else {
            secondary.push(candidate);
        }
    }

    ranked_available.sort_by(compare_maintenance_candidates);
    forced_priority.sort_by(|lhs, rhs| lhs.id.cmp(&rhs.id));
    high_frequency.sort_by(compare_maintenance_candidates);
    secondary.sort_by(compare_maintenance_candidates);

    let mut plans = Vec::new();
    for candidate in forced_priority {
        if maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now) {
            plans.push(MaintenanceDispatchPlan {
                account_id: candidate.id,
                tier: MaintenanceTier::Priority,
                sync_interval_secs: settings.primary_sync_interval_secs,
            });
        }
    }
    for candidate in high_frequency {
        if maintenance_plan_is_due(&candidate, MaintenanceTier::HighFrequency, settings, now) {
            plans.push(MaintenanceDispatchPlan {
                account_id: candidate.id,
                tier: MaintenanceTier::HighFrequency,
                sync_interval_secs: MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
            });
        }
    }
    for (index, candidate) in ranked_available.into_iter().enumerate() {
        let tier = if index < settings.priority_available_account_cap {
            MaintenanceTier::Priority
        } else {
            MaintenanceTier::Secondary
        };
        if maintenance_plan_is_due(&candidate, tier, settings, now) {
            plans.push(MaintenanceDispatchPlan {
                account_id: candidate.id,
                tier,
                sync_interval_secs: maintenance_interval_for_tier(tier, settings),
            });
        }
    }
    for candidate in secondary {
        if maintenance_plan_is_due(&candidate, MaintenanceTier::Secondary, settings, now) {
            plans.push(MaintenanceDispatchPlan {
                account_id: candidate.id,
                tier: MaintenanceTier::Secondary,
                sync_interval_secs: settings.secondary_sync_interval_secs,
            });
        }
    }

    plans
}

pub(crate) async fn find_existing_import_match(
    pool: &Pool<Sqlite>,
    chatgpt_account_id: &str,
    email: &str,
) -> Result<Option<UpstreamAccountRow>> {
    let account_id_matches = sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, created_at, updated_at
        FROM pool_upstream_accounts
        WHERE kind = ?1
          AND chatgpt_account_id = ?2
        ORDER BY updated_at DESC, id DESC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(chatgpt_account_id)
    .fetch_all(pool)
    .await?;
    if account_id_matches.len() > 1 {
        bail!(
            "multiple existing OAuth accounts match account_id {}",
            chatgpt_account_id
        );
    }
    if let Some(row) = account_id_matches.into_iter().next() {
        return Ok(Some(row));
    }

    let email_matches = sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, created_at, updated_at
        FROM pool_upstream_accounts
        WHERE kind = ?1
          AND lower(trim(COALESCE(email, ''))) = lower(trim(?2))
        ORDER BY updated_at DESC, id DESC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(email)
    .fetch_all(pool)
    .await?;
    if email_matches.len() > 1 {
        bail!("multiple existing OAuth accounts match email {}", email);
    }
    Ok(email_matches.into_iter().next())
}

pub(crate) async fn probe_imported_oauth_credentials(
    state: &AppState,
    imported: &NormalizedImportedOauthCredentials,
    refresh_scope: &ForwardProxyRouteScope,
    usage_scope: &ForwardProxyRouteScope,
) -> Result<ImportedOauthProbeOutcome, anyhow::Error> {
    let mut credentials = imported.credentials.clone();
    let mut claims = imported.claims.clone();
    let mut token_expires_at = imported.token_expires_at.clone();
    let expires_at = parse_rfc3339_utc(&token_expires_at);
    let refresh_due = expires_at
        .map(|expires| {
            expires
                <= Utc::now()
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        })
        .unwrap_or(true);

    if refresh_due {
        let response = refresh_oauth_tokens_for_required_scope(
            state,
            refresh_scope,
            &credentials.refresh_token,
        )
        .await?;
        credentials.access_token = response.access_token;
        if let Some(refresh_token) = response.refresh_token {
            credentials.refresh_token = refresh_token;
        }
        if let Some(id_token) = response.id_token {
            credentials.id_token = id_token;
            claims = parse_chatgpt_jwt_claims(&credentials.id_token)?;
            claims.email = claims.email.or_else(|| Some(imported.email.clone()));
            claims.chatgpt_account_id = claims
                .chatgpt_account_id
                .or_else(|| Some(imported.chatgpt_account_id.clone()));
        }
        credentials.token_type = response.token_type;
        token_expires_at =
            format_utc_iso(Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)));
    }

    let usage_result = fetch_usage_snapshot_via_forward_proxy(
        state,
        usage_scope,
        &state.config,
        &credentials.access_token,
        claims
            .chatgpt_account_id
            .as_deref()
            .or(Some(imported.chatgpt_account_id.as_str())),
    )
    .await;
    let (snapshot, usage_snapshot_warning) = match usage_result {
        Ok(snapshot) => (Some(snapshot), None),
        Err(err) if is_import_invalid_error_message(&err.to_string()) => return Err(err),
        Err(err) if err.to_string().contains("401") || err.to_string().contains("403") => {
            let response = refresh_oauth_tokens_for_required_scope(
                state,
                refresh_scope,
                &credentials.refresh_token,
            )
            .await?;
            credentials.access_token = response.access_token;
            if let Some(refresh_token) = response.refresh_token {
                credentials.refresh_token = refresh_token;
            }
            if let Some(id_token) = response.id_token {
                credentials.id_token = id_token;
                claims = parse_chatgpt_jwt_claims(&credentials.id_token)?;
                claims.email = claims.email.or_else(|| Some(imported.email.clone()));
                claims.chatgpt_account_id = claims
                    .chatgpt_account_id
                    .or_else(|| Some(imported.chatgpt_account_id.clone()));
            }
            credentials.token_type = response.token_type;
            token_expires_at =
                format_utc_iso(Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)));
            match fetch_usage_snapshot_via_forward_proxy(
                state,
                usage_scope,
                &state.config,
                &credentials.access_token,
                claims
                    .chatgpt_account_id
                    .as_deref()
                    .or(Some(imported.chatgpt_account_id.as_str())),
            )
            .await
            {
                Ok(snapshot) => (Some(snapshot), None),
                Err(retry_err)
                    if !is_import_invalid_error_message(&retry_err.to_string())
                        && !retry_err.to_string().contains("401")
                        && !retry_err.to_string().contains("403") =>
                {
                    (
                        None,
                        Some(format!(
                            "usage snapshot unavailable during validation: {retry_err}"
                        )),
                    )
                }
                Err(retry_err) => return Err(retry_err),
            }
        }
        Err(err) => (
            None,
            Some(format!(
                "usage snapshot unavailable during validation: {err}"
            )),
        ),
    };

    Ok(ImportedOauthProbeOutcome {
        token_expires_at,
        credentials,
        claims,
        usage_snapshot: snapshot.clone(),
        exhausted: snapshot
            .as_ref()
            .is_some_and(imported_snapshot_is_exhausted),
        usage_snapshot_warning,
    })
}
