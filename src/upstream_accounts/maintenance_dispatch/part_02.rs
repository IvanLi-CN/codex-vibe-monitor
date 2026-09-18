const MAINTENANCE_CANDIDATES_RANKED_BEFORE_QUERY: &str = r#"
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
              AND COALESCE(account.deleted_at, '') = ''
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
        "#;

pub(crate) async fn load_maintenance_candidates_ranked_before(
    pool: &Pool<Sqlite>,
    candidate: &MaintenanceCandidateRow,
    offset: usize,
    limit: usize,
) -> Result<Vec<MaintenanceCandidateRow>> {
    let secondary_used_percent = candidate.secondary_used_percent.unwrap_or(100.0);
    let primary_used_percent = candidate.primary_used_percent.unwrap_or(100.0);
    let last_synced_sort_key = candidate.last_synced_at.as_deref().unwrap_or("");
    sqlx::query_as::<_, MaintenanceCandidateRow>(MAINTENANCE_CANDIDATES_RANKED_BEFORE_QUERY)
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
            if !maintenance_candidate_counts_toward_available_priority_slot(
                other,
                state.config.upstream_accounts_refresh_lead_time,
                now,
            ) {
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
    if maintenance_candidate_blocks_upstream_rejected_cooldown(&candidate, now) {
        return Ok(None);
    }
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
    forced_priority.sort_by_key(|lhs| lhs.id);
    high_frequency.sort_by(compare_maintenance_candidates);
    secondary.sort_by(compare_maintenance_candidates);

    let mut plans = Vec::new();
    let mut available_priority_slots_used = 0usize;
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
    for candidate in ranked_available {
        let counts_toward_priority_slot =
            maintenance_candidate_counts_toward_available_priority_slot(
                &candidate,
                refresh_lead_time,
                now,
            );
        let tier = if counts_toward_priority_slot
            && available_priority_slots_used < settings.priority_available_account_cap
        {
            MaintenanceTier::Priority
        } else {
            MaintenanceTier::Secondary
        };
        if counts_toward_priority_slot {
            available_priority_slots_used += 1;
        }
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
