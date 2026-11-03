use super::*;
pub(crate) async fn load_upstream_account_summaries(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<Vec<UpstreamAccountSummary>> {
    load_upstream_account_summaries_for_query(pool, config, &ListUpstreamAccountsQuery::default())
        .await
}

pub(crate) async fn load_upstream_account_summaries_for_query(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    params: &ListUpstreamAccountsQuery,
) -> Result<Vec<UpstreamAccountSummary>> {
    let duplicate_info_map = load_duplicate_info_map(pool).await?;
    let mut normalized_tag_ids = params
        .tag_ids
        .iter()
        .copied()
        .filter(|tag_id| *tag_id > 0)
        .collect::<Vec<_>>();
    normalized_tag_ids.sort_unstable();
    normalized_tag_ids.dedup();
    let mut normalized_group_exact = params
        .group_exact
        .iter()
        .filter_map(|value| normalize_optional_text(Some(value.clone())))
        .collect::<Vec<_>>();
    normalized_group_exact.sort();
    normalized_group_exact.dedup();
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS} FROM pool_upstream_accounts"
    ));
    query.push(" WHERE COALESCE(deleted_at, '') = ''");

    if let Some(kind) = params.kind.as_deref() {
        query.push(" AND kind = ").push_bind(kind);
    }

    if params.group_ungrouped.unwrap_or(false) {
        query.push(" AND (NULLIF(TRIM(COALESCE(group_name, '')), '') IS NULL");
        query.push(" OR TRIM(COALESCE(group_name, '')) = ");
        query.push_bind(DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME);
        query.push(")");
    } else if !normalized_group_exact.is_empty() {
        query.push(" AND TRIM(COALESCE(group_name, '')) IN (");
        {
            let mut separated = query.separated(", ");
            for group_exact in &normalized_group_exact {
                separated.push_bind(group_exact);
            }
        }
        query.push(")");
    } else if let Some(group_search) = params
        .group_search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        query
            .push(" AND LOWER(TRIM(COALESCE(group_name, ''))) LIKE ")
            .push_bind(format!("%{}%", group_search.to_lowercase()));
    }

    if !normalized_tag_ids.is_empty() {
        query.push(" AND id IN (SELECT link.account_id FROM pool_upstream_account_tags link WHERE link.tag_id IN (");
        {
            let mut separated = query.separated(", ");
            for tag_id in &normalized_tag_ids {
                separated.push_bind(tag_id);
            }
        }
        query
            .push(") GROUP BY link.account_id HAVING COUNT(DISTINCT link.tag_id) = ")
            .push_bind(normalized_tag_ids.len() as i64)
            .push(")");
    }

    let rows = query
        .push(" ORDER BY updated_at DESC, id DESC")
        .build_query_as::<UpstreamAccountRow>()
        .fetch_all(pool)
        .await?;
    let now = Utc::now();
    let account_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let tag_map = load_account_tag_map(pool, &account_ids).await?;
    let active_conversation_count_map =
        load_account_active_conversation_count_map(pool, &account_ids, now).await?;
    let latest_sample_map = load_latest_usage_sample_map(pool, &account_ids).await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let latest = latest_sample_map.get(&row.id);
        let tags = tag_map.get(&row.id).cloned().unwrap_or_default();
        items.push(build_summary_from_row(
            &row,
            latest,
            row.last_activity_at.clone(),
            tags,
            duplicate_info_map.get(&row.id).cloned(),
            active_conversation_count_map
                .get(&row.id)
                .copied()
                .unwrap_or_default(),
            now,
        ));
    }
    apply_effective_routing_rules_to_summaries(pool, config, &mut items).await?;
    Ok(items)
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LatestUpstreamAccountSampleByAccountRow {
    account_id: i64,
    sample_id: i64,
    captured_at: String,
    limit_id: Option<String>,
    limit_name: Option<String>,
    account_plan_type: Option<String>,
    account_plan_type_observed_at: Option<String>,
    primary_used_percent: Option<f64>,
    primary_window_minutes: Option<i64>,
    primary_resets_at: Option<String>,
    secondary_used_percent: Option<f64>,
    secondary_window_minutes: Option<i64>,
    secondary_resets_at: Option<String>,
    credits_has_credits: Option<i64>,
    credits_unlimited: Option<i64>,
    credits_balance: Option<String>,
    reserve_status: Option<String>,
    reserve_allowed: Option<i64>,
    reserve_limit_reached: Option<i64>,
    reserve_normal_model_slug: Option<String>,
    reserve_metered_feature: Option<String>,
    reserve_binding_source: Option<String>,
    reserve_primary_used_percent: Option<f64>,
    reserve_primary_window_minutes: Option<i64>,
    reserve_primary_resets_at: Option<String>,
    reserve_primary_available_percent: Option<f64>,
    reserve_secondary_used_percent: Option<f64>,
    reserve_secondary_window_minutes: Option<i64>,
    reserve_secondary_resets_at: Option<String>,
    reserve_secondary_available_percent: Option<f64>,
    reserve_captured_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LatestUpstreamAccountPlanTypeRow {
    account_id: i64,
    captured_at: String,
    plan_type: Option<String>,
}

pub(crate) async fn load_latest_usage_sample_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, UpstreamAccountSampleRow>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut latest_sample_query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            sample.account_id,
            sample.id AS sample_id,
            sample.captured_at,
            sample.limit_id,
            sample.limit_name,
            NULLIF(TRIM(account.plan_type), '') AS account_plan_type,
            account.plan_type_observed_at AS account_plan_type_observed_at,
            sample.primary_used_percent,
            sample.primary_window_minutes,
            sample.primary_resets_at,
            sample.secondary_used_percent,
            sample.secondary_window_minutes,
            sample.secondary_resets_at,
            sample.credits_has_credits,
            sample.credits_unlimited,
            sample.credits_balance,
            sample.reserve_status,
            sample.reserve_allowed,
            sample.reserve_limit_reached,
            sample.reserve_normal_model_slug,
            sample.reserve_metered_feature,
            sample.reserve_binding_source,
            sample.reserve_primary_used_percent,
            sample.reserve_primary_window_minutes,
            sample.reserve_primary_resets_at,
            sample.reserve_primary_available_percent,
            sample.reserve_secondary_used_percent,
            sample.reserve_secondary_window_minutes,
            sample.reserve_secondary_resets_at,
            sample.reserve_secondary_available_percent,
            sample.reserve_captured_at
        FROM pool_upstream_account_limit_samples sample
        INNER JOIN (
            SELECT account_id, MAX(captured_at) AS captured_at
            FROM pool_upstream_account_limit_samples
            WHERE account_id IN (
        "#,
    );
    {
        let mut separated = latest_sample_query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    latest_sample_query.push(
        r#"
            )
            GROUP BY account_id
        ) latest
            ON latest.account_id = sample.account_id
           AND latest.captured_at = sample.captured_at
        INNER JOIN pool_upstream_accounts account ON account.id = sample.account_id
        ORDER BY sample.account_id ASC, sample.id DESC
        "#,
    );
    let rows = latest_sample_query
        .build_query_as::<LatestUpstreamAccountSampleByAccountRow>()
        .fetch_all(pool)
        .await?;
    let mut latest_rows = rows.into_iter().fold(
        HashMap::<i64, LatestUpstreamAccountSampleByAccountRow>::new(),
        |mut acc, row| {
            acc.entry(row.account_id).or_insert(row);
            acc
        },
    );

    let mut latest_plan_type_query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            sample.account_id,
            sample.captured_at,
            NULLIF(TRIM(sample.plan_type), '') AS plan_type
        FROM pool_upstream_account_limit_samples sample
        INNER JOIN (
            SELECT account_id, MAX(captured_at) AS captured_at
            FROM pool_upstream_account_limit_samples
            WHERE account_id IN (
        "#,
    );
    {
        let mut separated = latest_plan_type_query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    latest_plan_type_query.push(
        r#"
            )
              AND plan_type IS NOT NULL
              AND TRIM(plan_type) <> ''
            GROUP BY account_id
        ) latest
            ON latest.account_id = sample.account_id
           AND latest.captured_at = sample.captured_at
        WHERE sample.plan_type IS NOT NULL
          AND TRIM(sample.plan_type) <> ''
        ORDER BY sample.account_id ASC, sample.id DESC
        "#,
    );
    let latest_plan_type_rows = latest_plan_type_query
        .build_query_as::<LatestUpstreamAccountPlanTypeRow>()
        .fetch_all(pool)
        .await?;
    let latest_plan_type_map = latest_plan_type_rows.into_iter().fold(
        HashMap::<i64, LatestUpstreamAccountPlanTypeRow>::new(),
        |mut acc, row| {
            acc.entry(row.account_id).or_insert(row);
            acc
        },
    );

    Ok(latest_rows
        .drain()
        .map(|(account_id, row)| {
            let account_plan_type = normalize_plan_type(row.account_plan_type.as_deref());
            let sample_plan_type_row = latest_plan_type_map.get(&account_id);
            let sample_plan_type = sample_plan_type_row
                .and_then(|value| normalize_plan_type(value.plan_type.as_deref()));
            let account_plan_is_current = account_plan_type.is_some()
                && parse_rfc3339_utc(
                    sample_plan_type_row
                        .map(|value| value.captured_at.as_str())
                        .unwrap_or_default(),
                )
                .zip(
                    row.account_plan_type_observed_at
                        .as_deref()
                        .and_then(parse_rfc3339_utc),
                )
                .is_some_and(|(sample_captured_at, account_observed_at)| {
                    account_observed_at >= sample_captured_at
                });
            let effective_plan_type = if account_plan_is_current {
                account_plan_type.clone().or(sample_plan_type.clone())
            } else {
                sample_plan_type.clone().or(account_plan_type.clone())
            };
            (
                account_id,
                UpstreamAccountSampleRow {
                    id: row.sample_id,
                    captured_at: row.captured_at,
                    limit_id: row.limit_id,
                    limit_name: row.limit_name,
                    plan_type: effective_plan_type,
                    primary_used_percent: row.primary_used_percent,
                    primary_window_minutes: row.primary_window_minutes,
                    primary_resets_at: row.primary_resets_at,
                    secondary_used_percent: row.secondary_used_percent,
                    secondary_window_minutes: row.secondary_window_minutes,
                    secondary_resets_at: row.secondary_resets_at,
                    credits_has_credits: row.credits_has_credits,
                    credits_unlimited: row.credits_unlimited,
                    credits_balance: row.credits_balance,
                    reserve_status: row.reserve_status,
                    reserve_allowed: row.reserve_allowed,
                    reserve_limit_reached: row.reserve_limit_reached,
                    reserve_normal_model_slug: row.reserve_normal_model_slug,
                    reserve_metered_feature: row.reserve_metered_feature,
                    reserve_binding_source: row.reserve_binding_source,
                    reserve_primary_used_percent: row.reserve_primary_used_percent,
                    reserve_primary_window_minutes: row.reserve_primary_window_minutes,
                    reserve_primary_resets_at: row.reserve_primary_resets_at,
                    reserve_primary_available_percent: row.reserve_primary_available_percent,
                    reserve_secondary_used_percent: row.reserve_secondary_used_percent,
                    reserve_secondary_window_minutes: row.reserve_secondary_window_minutes,
                    reserve_secondary_resets_at: row.reserve_secondary_resets_at,
                    reserve_secondary_available_percent: row.reserve_secondary_available_percent,
                    reserve_captured_at: row.reserve_captured_at,
                },
            )
        })
        .collect())
}

pub(crate) async fn load_upstream_account_rows_by_ids(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<Vec<UpstreamAccountRow>> {
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS} FROM pool_upstream_accounts WHERE id IN ("
    ));
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(") AND COALESCE(deleted_at, '') = '' ORDER BY updated_at DESC, id DESC");
    query
        .build_query_as::<UpstreamAccountRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_upstream_account_window_usage_summaries(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    account_ids: &[i64],
) -> Result<Vec<UpstreamAccountSummary>> {
    let rows = load_upstream_account_rows_by_ids(pool, account_ids).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let latest_sample_map = load_latest_usage_sample_map(pool, account_ids).await?;
    let now = Utc::now();
    let mut summaries = rows
        .into_iter()
        .map(|row| {
            build_summary_from_row(
                &row,
                latest_sample_map.get(&row.id),
                row.last_activity_at.clone(),
                Vec::new(),
                None,
                0,
                now,
            )
        })
        .collect::<Vec<_>>();
    apply_effective_routing_rules_to_summaries(pool, config, &mut summaries).await?;
    Ok(summaries)
}

pub(crate) async fn build_bulk_upstream_account_sync_pending_rows(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<Vec<BulkUpstreamAccountSyncRow>> {
    let mut rows = Vec::with_capacity(account_ids.len());
    for account_id in account_ids {
        let display_name = load_upstream_account_row(pool, *account_id)
            .await?
            .map(|row| row.display_name)
            .unwrap_or_else(|| format!("Account {account_id}"));
        rows.push(BulkUpstreamAccountSyncRow {
            account_id: *account_id,
            display_name,
            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING.to_string(),
            detail: None,
        });
    }
    Ok(rows)
}

pub(crate) async fn apply_bulk_upstream_account_action(
    state: Arc<AppState>,
    account_id: i64,
    action: &str,
    group_name: Option<String>,
) -> Result<(), (StatusCode, String)> {
    if action == BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP {
        let kind = sqlx::query_scalar::<_, String>(
            "SELECT kind FROM pool_upstream_accounts WHERE id = ?1 AND COALESCE(deleted_at, '') = ''",
        )
        .bind(account_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
        if kind.as_deref() == Some(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX) {
            return Err((
                StatusCode::BAD_REQUEST,
                "API Key accounts cannot be assigned to groups".to_string(),
            ));
        }
    }
    let payload = match action {
        BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE => UpdateUpstreamAccountRequest {
            display_name: None,
            email: OptionalField::Missing,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            upstream_base_url: OptionalField::Missing,
            bound_proxy_keys: OptionalField::Missing,
            enabled: Some(true),
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
            routing_rule: None,
            ..UpdateUpstreamAccountRequest::default()
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE => UpdateUpstreamAccountRequest {
            display_name: None,
            email: OptionalField::Missing,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            upstream_base_url: OptionalField::Missing,
            bound_proxy_keys: OptionalField::Missing,
            enabled: Some(false),
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
            routing_rule: None,
            ..UpdateUpstreamAccountRequest::default()
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP => UpdateUpstreamAccountRequest {
            display_name: None,
            email: OptionalField::Missing,
            group_name,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            upstream_base_url: OptionalField::Missing,
            bound_proxy_keys: OptionalField::Missing,
            enabled: None,
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
            routing_rule: None,
            ..UpdateUpstreamAccountRequest::default()
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_DELETE => {
            state
                .upstream_accounts
                .account_ops
                .run_delete_account(state.clone(), account_id)
                .await?;
            return Ok(());
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "unsupported bulk action".to_string(),
            ));
        }
    };

    state
        .upstream_accounts
        .account_ops
        .run_update_account(state.clone(), account_id, payload)
        .await?;
    Ok(())
}

pub(crate) async fn has_ungrouped_upstream_accounts(
    pool: &Pool<Sqlite>,
    kind: Option<&str>,
) -> Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE COALESCE(deleted_at, '') = ''
          AND (?1 IS NULL OR (kind = ?1 AND kind = 'oauth_codex'))
          AND NULLIF(TRIM(COALESCE(group_name, '')), '') IS NULL
        "#,
    )
    .bind(kind)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

pub(crate) async fn load_upstream_account_detail(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    load_upstream_account_detail_with_options(
        pool,
        id,
        LoadUpstreamAccountDetailOptions {
            include_recent_actions: true,
        },
    )
    .await
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LoadUpstreamAccountDetailOptions {
    pub(crate) include_recent_actions: bool,
}

pub(crate) async fn load_upstream_account_detail_with_options(
    pool: &Pool<Sqlite>,
    id: i64,
    options: LoadUpstreamAccountDetailOptions,
) -> Result<Option<UpstreamAccountDetail>> {
    let Some(row) = load_upstream_account_row(pool, id).await? else {
        return Ok(None);
    };
    let latest = load_latest_usage_sample(pool, row.id).await?;
    let tags = load_account_tag_map(pool, &[row.id])
        .await?
        .remove(&row.id)
        .unwrap_or_default();
    let history_rows = sqlx::query_as::<_, UpstreamAccountSampleRow>(
        r#"
        SELECT
            id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance,
            reserve_status, reserve_allowed, reserve_limit_reached,
            reserve_normal_model_slug, reserve_metered_feature, reserve_binding_source,
            reserve_primary_used_percent, reserve_primary_window_minutes, reserve_primary_resets_at,
            reserve_primary_available_percent,
            reserve_secondary_used_percent, reserve_secondary_window_minutes, reserve_secondary_resets_at,
            reserve_secondary_available_percent,
            reserve_captured_at
        FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1
        ORDER BY captured_at DESC, id DESC
        LIMIT 128
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let mut history = history_rows
        .into_iter()
        .map(|sample| UpstreamAccountHistoryPoint {
            captured_at: sample.captured_at,
            primary_used_percent: sample.primary_used_percent,
            secondary_used_percent: sample.secondary_used_percent,
            credits_balance: sample.credits_balance,
        })
        .collect::<Vec<_>>();
    history.reverse();
    let recent_action_rows = if options.include_recent_actions {
        sqlx::query_as::<_, UpstreamAccountActionEventRow>(
            r#"
            SELECT
                event.id,
                event.occurred_at,
                event.action,
                event.source,
                COALESCE(event.account_display_name, account.display_name) AS account_display_name,
                COALESCE(event.account_group_name, account.group_name) AS account_group_name,
                event.forward_proxy_key,
                event.forward_proxy_display_name,
                event.forward_proxy_egress_ip,
                event.result,
                event.result_description,
                event.reason_code,
                event.reason_message,
                event.http_status,
                event.failure_kind,
                event.invoke_id,
                attempts.attempt_public_id,
                event.sticky_key,
                COALESCE(
                    event.model,
                    attempts.request_model,
                    (
                        SELECT COALESCE(
                            NULLIF(TRIM(CASE
                                WHEN json_valid(fallback_invocation.payload)
                                    THEN CAST(json_extract(fallback_invocation.payload, '$.requestModel') AS TEXT)
                            END), ''),
                            NULLIF(TRIM(fallback_invocation.model), '')
                        )
                        FROM codex_invocations fallback_invocation
                        WHERE fallback_invocation.invoke_id = event.invoke_id
                          AND ABS(
                                julianday(
                                    fallback_invocation.occurred_at,
                                    CASE WHEN instr(fallback_invocation.occurred_at, 'T') > 0
                                        THEN '+0 hours' ELSE '-8 hours' END
                                ) -
                                julianday(
                                    COALESCE(attempts.occurred_at, event.occurred_at),
                                    CASE WHEN instr(COALESCE(attempts.occurred_at, event.occurred_at), 'T') > 0
                                        THEN '+0 hours' ELSE '-8 hours' END
                                )
                              ) = (
                                SELECT MIN(ABS(
                                    julianday(
                                        candidate.occurred_at,
                                        CASE WHEN instr(candidate.occurred_at, 'T') > 0
                                            THEN '+0 hours' ELSE '-8 hours' END
                                    ) -
                                    julianday(
                                        COALESCE(attempts.occurred_at, event.occurred_at),
                                        CASE WHEN instr(COALESCE(attempts.occurred_at, event.occurred_at), 'T') > 0
                                            THEN '+0 hours' ELSE '-8 hours' END
                                    )
                                ))
                                FROM codex_invocations candidate
                                WHERE candidate.invoke_id = event.invoke_id
                              )
                        ORDER BY fallback_invocation.id DESC
                        LIMIT 1
                    )
                ) AS model,
                event.model_route_state_before,
                event.model_route_state_after,
                event.model_route_priority_before,
                event.model_route_priority_after,
                event.model_route_failure_count,
                event.model_route_cooldown_until,
                (
                    SELECT CASE
                        WHEN json_valid(fallback_invocation.payload)
                         AND json_type(fallback_invocation.payload, '$.blockedBinding') = 'object'
                            THEN json_extract(fallback_invocation.payload, '$.blockedBinding')
                    END
                    FROM codex_invocations fallback_invocation
                    WHERE fallback_invocation.invoke_id = event.invoke_id
                      AND ABS(
                            julianday(
                                fallback_invocation.occurred_at,
                                CASE WHEN instr(fallback_invocation.occurred_at, 'T') > 0
                                    THEN '+0 hours' ELSE '-8 hours' END
                            ) -
                            julianday(
                                COALESCE(attempts.occurred_at, event.occurred_at),
                                CASE WHEN instr(COALESCE(attempts.occurred_at, event.occurred_at), 'T') > 0
                                    THEN '+0 hours' ELSE '-8 hours' END
                            )
                          ) = (
                            SELECT MIN(ABS(
                                julianday(
                                    candidate.occurred_at,
                                    CASE WHEN instr(candidate.occurred_at, 'T') > 0
                                        THEN '+0 hours' ELSE '-8 hours' END
                                ) -
                                julianday(
                                    COALESCE(attempts.occurred_at, event.occurred_at),
                                    CASE WHEN instr(COALESCE(attempts.occurred_at, event.occurred_at), 'T') > 0
                                        THEN '+0 hours' ELSE '-8 hours' END
                                )
                            ))
                            FROM codex_invocations candidate
                            WHERE candidate.invoke_id = event.invoke_id
                          )
                    ORDER BY fallback_invocation.id DESC
                    LIMIT 1
                ) AS blocked_binding_json,
                event.created_at
            FROM pool_upstream_account_events event
            INNER JOIN pool_upstream_accounts account ON account.id = event.account_id
            LEFT JOIN pool_upstream_request_attempts attempts ON attempts.id = event.attempt_id
            WHERE event.account_id = ?1
            ORDER BY event.occurred_at DESC, event.id DESC
            LIMIT 20
            "#,
        )
        .bind(id)
        .fetch_all(pool)
        .await?
    } else {
        Vec::new()
    };
    let credential_refresh = load_upstream_account_refresh_status(
        pool,
        row.id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await?;
    let usage_window_refresh = load_upstream_account_refresh_status(
        pool,
        row.id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
    )
    .await?;
    let credential_refresh_history = load_upstream_account_refresh_history(pool, row.id, 5).await?;

    let duplicate_info = load_duplicate_info_for_account(pool, row.id).await?;
    let model_catalog = load_upstream_account_model_catalog(pool, row.id).await?;
    let now = Utc::now();
    let active_conversation_count =
        load_account_active_conversation_count_map(pool, &[row.id], now)
            .await?
            .get(&row.id)
            .copied()
            .unwrap_or_default();
    let summary = build_summary_from_row(
        &row,
        latest.as_ref(),
        row.last_activity_at.clone(),
        tags,
        duplicate_info,
        active_conversation_count,
        now,
    );
    let token_lifecycle = build_token_lifecycle(&row, &credential_refresh, now);
    Ok(Some(UpstreamAccountDetail {
        summary,
        note: row.note,
        upstream_base_url: row.upstream_base_url,
        chatgpt_user_id: row.chatgpt_user_id,
        verified_email: row.verified_email,
        last_refreshed_at: row.last_refreshed_at,
        credential_refresh,
        usage_window_refresh,
        token_lifecycle,
        credential_refresh_history,
        history,
        recent_actions: recent_action_rows
            .iter()
            .map(build_action_event_from_row)
            .collect(),
        model_mappings: decode_model_mappings_json(row.model_mappings_json.as_deref()),
        model_catalog,
        model_routing_states: load_model_routing_states(pool, row.id).await?,
        routing_state_version: None,
    }))
}

pub(crate) async fn load_upstream_account_detail_with_actual_usage(
    state: &AppState,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    load_upstream_account_detail_with_actual_usage_options(
        state,
        id,
        LoadUpstreamAccountDetailOptions {
            include_recent_actions: true,
        },
    )
    .await
}

pub(crate) async fn load_upstream_account_detail_with_actual_usage_options(
    state: &AppState,
    id: i64,
    options: LoadUpstreamAccountDetailOptions,
) -> Result<Option<UpstreamAccountDetail>> {
    let mut detail =
        match load_upstream_account_detail_with_options(&state.pool, id, options).await? {
            Some(detail) => detail,
            None => return Ok(None),
        };
    enrich_window_actual_usage_for_summaries(state, std::slice::from_mut(&mut detail.summary))
        .await?;
    apply_effective_routing_rules_to_summaries(
        &state.pool,
        &state.config,
        std::slice::from_mut(&mut detail.summary),
    )
    .await?;
    enrich_transport_decode_sticky_escape_routing_block_reasons(
        state,
        std::slice::from_mut(&mut detail.summary),
    )
    .await?;
    let group =
        load_canonicalized_upstream_account_group(state, detail.summary.group_name.as_deref())
            .await?;
    if group.as_ref().is_some_and(|value| value.node_shunt_enabled) {
        let groups = group.into_iter().collect::<Vec<_>>();
        enrich_node_shunt_routing_block_reasons(state, std::slice::from_mut(&mut detail.summary))
            .await?;
        enrich_current_forward_proxy_for_summaries(
            state,
            &groups,
            std::slice::from_mut(&mut detail.summary),
        )
        .await?;
    } else {
        enrich_current_forward_proxy_for_non_node_shunt_detail(
            state,
            group.as_ref(),
            &mut detail.summary,
        )
        .await?;
    }
    detail.routing_state_version = current_routing_state_version(state);
    Ok(Some(detail))
}

pub(crate) async fn apply_effective_routing_rules_to_summaries(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    summaries: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if summaries.is_empty() {
        return Ok(());
    }
    let account_ids = summaries
        .iter()
        .map(|summary| summary.id)
        .collect::<Vec<_>>();
    let rules = load_effective_routing_rules_for_accounts(pool, &account_ids).await?;
    let root_timeouts = resolve_pool_routing_timeouts(pool, config).await?;
    for summary in summaries {
        if let Some(rule) = rules.get(&summary.id).cloned() {
            let mut rule = rule;
            apply_root_routing_timeout_defaults(&mut rule, &root_timeouts);
            summary.effective_routing_rule = rule;
        }
    }
    Ok(())
}

pub(crate) async fn enrich_transport_decode_sticky_escape_routing_block_reasons(
    state: &AppState,
    items: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let account_ids = items.iter().map(|item| item.id).collect::<Vec<_>>();
    let states = load_transport_decode_sticky_escape_states(&state.pool, &account_ids).await?;
    for item in items {
        let Some(state) = states.get(&item.id) else {
            continue;
        };
        if !item.enabled
            || item.sync_state == UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING
            || item.health_status != UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
            || matches!(
                item.work_status.as_str(),
                UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
                    | UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
            )
        {
            continue;
        }
        item.work_status = UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED.to_string();
        item.health_status = UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string();
        item.routing_block_reason_code =
            Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_RECENT_UPSTREAM_STREAM_ERRORS.to_string());
        item.routing_block_reason_message = Some(
            UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_RECENT_UPSTREAM_STREAM_ERRORS_MESSAGE.to_string(),
        );
        item.routing_block_until = Some(format_utc_iso(state.until));
    }
    Ok(())
}

pub(crate) async fn load_upstream_account_row(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    let mut conn = pool.acquire().await?;
    load_upstream_account_row_conn(&mut conn, id).await
}

pub(crate) async fn load_upstream_account_row_conn(
    conn: &mut SqliteConnection,
    id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    sqlx::query_as::<_, UpstreamAccountRow>(&format!(
        "SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS} FROM pool_upstream_accounts WHERE id = ?1 AND COALESCE(deleted_at, '') = '' LIMIT 1"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_upstream_account_row_by_external_identity(
    pool: &Pool<Sqlite>,
    external_client_id: &str,
    external_source_account_id: &str,
) -> Result<Option<UpstreamAccountRow>> {
    let mut conn = pool.acquire().await?;
    load_upstream_account_row_by_external_identity_conn(
        &mut conn,
        external_client_id,
        external_source_account_id,
    )
    .await
}

pub(crate) async fn load_upstream_account_row_by_external_identity_conn(
    conn: &mut SqliteConnection,
    external_client_id: &str,
    external_source_account_id: &str,
) -> Result<Option<UpstreamAccountRow>> {
    sqlx::query_as::<_, UpstreamAccountRow>(&format!(
        "SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS} FROM pool_upstream_accounts \
         WHERE external_client_id = ?1 AND external_source_account_id = ?2 \
           AND COALESCE(deleted_at, '') = '' \
         ORDER BY id ASC LIMIT 1"
    ))
    .bind(external_client_id)
    .bind(external_source_account_id)
    .fetch_optional(conn)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_latest_usage_sample(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<UpstreamAccountSampleRow>> {
    Ok(load_latest_usage_sample_map(pool, &[account_id])
        .await?
        .remove(&account_id))
}

pub(crate) fn build_summary_from_row(
    row: &UpstreamAccountRow,
    sample: Option<&UpstreamAccountSampleRow>,
    last_activity_at: Option<String>,
    tags: Vec<AccountTagSummary>,
    duplicate_info: Option<DuplicateInfo>,
    active_conversation_count: i64,
    now: DateTime<Utc>,
) -> UpstreamAccountSummary {
    let local_limits = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        Some(LocalLimitSnapshot {
            primary_limit: row.local_primary_limit,
            secondary_limit: row.local_secondary_limit,
            limit_unit: row
                .local_limit_unit
                .clone()
                .unwrap_or_else(|| DEFAULT_API_KEY_LIMIT_UNIT.to_string()),
        })
    } else {
        None
    };
    let primary_window = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        build_api_key_window(
            row.local_primary_limit,
            row.local_limit_unit.as_deref(),
            300,
        )
    } else {
        sample.and_then(|value| {
            build_window_snapshot(
                value.primary_used_percent,
                value.primary_window_minutes,
                value.primary_resets_at.as_deref(),
            )
        })
    };
    let secondary_window = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        build_api_key_window(
            row.local_secondary_limit,
            row.local_limit_unit.as_deref(),
            10_080,
        )
    } else {
        sample.and_then(|value| {
            build_window_snapshot(
                value.secondary_used_percent,
                value.secondary_window_minutes,
                value.secondary_resets_at.as_deref(),
            )
        })
    };
    let credits = sample.and_then(|value| {
        value
            .credits_has_credits
            .map(|has_credits| CreditsSnapshot {
                has_credits: has_credits != 0,
                unlimited: value.credits_unlimited.unwrap_or_default() != 0,
                balance: value.credits_balance.clone(),
            })
    });
    let reserve = (row.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX).then(|| {
        let captured_at = sample.and_then(|value| {
            value
                .reserve_captured_at
                .clone()
                .or_else(|| Some(value.captured_at.clone()))
        });
        let fresh = captured_at
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|captured_at| {
                let age_secs = now.signed_duration_since(captured_at).num_seconds();
                (0..=RESERVE_USAGE_SNAPSHOT_MAX_AGE_SECS).contains(&age_secs)
            });
        ReserveQuotaSnapshot {
            availability: if fresh {
                sample
                    .and_then(|value| value.reserve_status.clone())
                    .unwrap_or_else(|| RESERVE_AVAILABILITY_UNKNOWN.to_string())
            } else {
                RESERVE_AVAILABILITY_UNKNOWN.to_string()
            },
            allowed: sample
                .and_then(|value| value.reserve_allowed)
                .map(|value| value != 0),
            limit_reached: sample
                .and_then(|value| value.reserve_limit_reached)
                .map(|value| value != 0),
            normal_model_slug: sample.and_then(|value| value.reserve_normal_model_slug.clone()),
            metered_feature: sample.and_then(|value| value.reserve_metered_feature.clone()),
            binding_source: sample.and_then(|value| value.reserve_binding_source.clone()),
            primary_window: sample.and_then(|value| {
                build_window_snapshot(
                    value.reserve_primary_used_percent,
                    value.reserve_primary_window_minutes,
                    value.reserve_primary_resets_at.as_deref(),
                )
                .map(|mut window| {
                    window.available_percent = value.reserve_primary_available_percent;
                    window
                })
            }),
            secondary_window: sample.and_then(|value| {
                build_window_snapshot(
                    value.reserve_secondary_used_percent,
                    value.reserve_secondary_window_minutes,
                    value.reserve_secondary_resets_at.as_deref(),
                )
                .map(|mut window| {
                    window.available_percent = value.reserve_secondary_available_percent;
                    window
                })
            }),
            captured_at,
        }
    });
    let effective_routing_rule = build_effective_routing_rule(&tags);
    let status = effective_account_status(row);
    let enable_status = derive_upstream_account_enable_status(row.enabled != 0);
    let health_status = derive_upstream_account_health_status(
        &row.kind,
        row.enabled != 0,
        &row.status,
        row.last_error.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
    );
    let sync_state = derive_upstream_account_sync_state(row.enabled != 0, &row.status);
    let snapshot_exhausted = persisted_usage_sample_is_exhausted(sample);
    let work_status = derive_upstream_account_work_status(
        row.enabled != 0,
        &row.status,
        health_status,
        sync_state,
        snapshot_exhausted,
        row.cooldown_until.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
        row.temporary_route_failure_streak_started_at.as_deref(),
        row.last_selected_at.as_deref(),
        now,
    );
    let display_status = classify_upstream_account_display_status(
        &row.kind,
        row.enabled != 0,
        &row.status,
        row.last_error.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
    )
    .to_string();
    let compact_support = build_compact_support_state(row);

    UpstreamAccountSummary {
        id: row.id,
        kind: row.kind.clone(),
        provider: row.provider.clone(),
        display_name: row.display_name.clone(),
        group_name: row.group_name.clone(),
        is_mother: row.is_mother != 0,
        status,
        display_status,
        enabled: row.enabled != 0,
        work_status: work_status.to_string(),
        enable_status: enable_status.to_string(),
        health_status: health_status.to_string(),
        sync_state: sync_state.to_string(),
        email: row.email.clone(),
        chatgpt_account_id: row.chatgpt_account_id.clone(),
        plan_type: resolve_effective_plan_type(
            row.plan_type.as_deref(),
            sample.and_then(|value| value.plan_type.as_deref()),
        ),
        masked_api_key: row.masked_api_key.clone(),
        has_refresh_token: row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
            || row.has_refresh_token.unwrap_or(1) != 0,
        last_synced_at: row.last_synced_at.clone(),
        last_successful_sync_at: row.last_successful_sync_at.clone(),
        last_activity_at: last_activity_at
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .map(format_utc_iso)
            .or(last_activity_at),
        active_conversation_count,
        last_error: row.last_error.clone(),
        last_error_at: row.last_error_at.clone(),
        last_action: row.last_action.clone(),
        last_action_source: row.last_action_source.clone(),
        last_action_reason_code: row.last_action_reason_code.clone(),
        last_action_reason_message: row.last_action_reason_message.clone(),
        last_action_http_status: row
            .last_action_http_status
            .and_then(|value| u16::try_from(value).ok()),
        last_action_invoke_id: row.last_action_invoke_id.clone(),
        last_action_at: row.last_action_at.clone(),
        cooldown_until: row.cooldown_until.clone(),
        bound_proxy_keys: row.bound_proxy_keys(),
        current_forward_proxy_key: None,
        current_forward_proxy_display_name: None,
        current_forward_proxy_state: UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_UNCONFIGURED.to_string(),
        routing_block_reason_code: None,
        routing_block_reason_message: None,
        routing_block_until: None,
        token_expires_at: row.token_expires_at.clone(),
        primary_window,
        secondary_window,
        credits,
        reserve,
        local_limits,
        compact_support,
        duplicate_info,
        tags,
        effective_routing_rule,
        response_endpoint_capability: build_capability_state(
            row.response_endpoint_capability.as_deref(),
            row.response_endpoint_capability_observed_at.as_ref(),
            row.response_endpoint_capability_reason.as_ref(),
            row.policy_response_endpoint_capability_override.as_deref(),
        ),
        chat_completions_capability: build_capability_state(
            row.chat_completions_capability.as_deref(),
            row.chat_completions_capability_observed_at.as_ref(),
            row.chat_completions_capability_reason.as_ref(),
            row.policy_chat_completions_capability_override.as_deref(),
        ),
        image_endpoint_capability: build_capability_state(
            row.image_endpoint_capability.as_deref(),
            row.image_endpoint_capability_observed_at.as_ref(),
            row.image_endpoint_capability_reason.as_ref(),
            row.policy_image_endpoint_capability_override.as_deref(),
        ),
        response_image_tool_capability: build_capability_state(
            row.response_image_tool_capability.as_deref(),
            row.response_image_tool_capability_observed_at.as_ref(),
            row.response_image_tool_capability_reason.as_ref(),
            row.policy_response_image_tool_capability_override
                .as_deref(),
        ),
        codex_imagegen_capability: build_capability_state(
            row.codex_imagegen_capability.as_deref(),
            row.codex_imagegen_capability_observed_at.as_ref(),
            row.codex_imagegen_capability_reason.as_ref(),
            row.policy_codex_imagegen_capability_override.as_deref(),
        ),
        standalone_search_capability: build_capability_state(
            row.standalone_search_capability.as_deref(),
            row.standalone_search_capability_observed_at.as_ref(),
            row.standalone_search_capability_reason.as_ref(),
            row.policy_standalone_search_capability_override.as_deref(),
        ),
    }
}

pub(crate) fn apply_node_shunt_routing_block_reasons_to_summaries(
    items: &mut [UpstreamAccountSummary],
    assignments: &UpstreamAccountNodeShuntAssignments,
) {
    for item in items {
        let Some(group_name) = normalize_optional_text(item.group_name.clone()) else {
            continue;
        };
        if !assignments.group_slots.contains_key(&group_name) {
            continue;
        }
        if !assignments.eligible_account_ids.contains(&item.id) {
            continue;
        }
        if assignments.account_proxy_keys.contains_key(&item.id) {
            continue;
        }
        item.work_status = UPSTREAM_ACCOUNT_WORK_STATUS_IDLE.to_string();
        item.health_status = UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string();
        item.routing_block_reason_code =
            Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED.to_string());
        item.routing_block_reason_message =
            Some(group_node_shunt_unassigned_error_message().to_string());
        item.routing_block_until = None;
    }
}

pub(crate) async fn enrich_node_shunt_routing_block_reasons(
    state: &AppState,
    items: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let assignments = build_upstream_account_node_shunt_assignments(state).await?;
    apply_node_shunt_routing_block_reasons_to_summaries(items, &assignments);
    Ok(())
}

pub(crate) async fn load_canonicalized_upstream_account_groups(
    state: &AppState,
) -> Result<Vec<UpstreamAccountGroupSummary>> {
    let mut groups = load_upstream_account_groups(state).await?;
    for group in &mut groups {
        group.bound_proxy_keys =
            canonicalize_forward_proxy_bound_keys(state, &group.bound_proxy_keys).await?;
    }
    Ok(groups)
}

pub(crate) async fn load_canonicalized_upstream_account_group(
    state: &AppState,
    group_name: Option<&str>,
) -> Result<Option<UpstreamAccountGroupSummary>> {
    let normalized_group_name = group_name.map(str::trim).filter(|value| !value.is_empty());
    let Some(group_name) = normalized_group_name else {
        return Ok(None);
    };
    let metadata = load_group_metadata(&state.pool, Some(group_name)).await?;
    let routing_rule = load_group_routing_rule(&state.pool, group_name).await?;
    let (effective_timeouts, timeout_field_sources, _) =
        load_effective_request_path_timeouts_for_group(
            &state.pool,
            &state.config,
            Some(group_name),
        )
        .await?;
    Ok(Some(UpstreamAccountGroupSummary {
        group_name: group_name.to_string(),
        account_count: sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM pool_upstream_accounts
            WHERE deleted_at IS NULL
              AND kind = ?2
              AND TRIM(COALESCE(group_name, '')) = ?1
            "#,
        )
        .bind(group_name)
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .fetch_one(&state.pool)
        .await?,
        note: metadata.note.clone(),
        bound_proxy_keys: canonicalize_forward_proxy_bound_keys(state, &metadata.bound_proxy_keys)
            .await?,
        node_shunt_enabled: metadata.node_shunt_enabled,
        single_account_rotation_enabled: metadata.single_account_rotation_enabled,
        upstream_429_retry_enabled: metadata.upstream_429_retry_enabled,
        upstream_429_max_retries: metadata.upstream_429_max_retries,
        concurrency_limit: metadata.concurrency_limit,
        effective_timeouts,
        timeout_field_sources,
        routing_rule,
    }))
}

pub(crate) fn group_routing_rule_from_group_metadata(
    metadata: &UpstreamAccountGroupMetadata,
) -> GroupAccountRoutingRule {
    GroupAccountRoutingRule {
        allow_cut_out: true,
        allow_cut_in: true,
        priority_tier: TagPriorityTier::Normal,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Some(CodexImagegenRewriteMode::KeepOriginal),
        request_compression_algorithm: None,
        oauth_request_compression_algorithm: None,
        concurrency_limit: metadata.concurrency_limit,
        upstream_429_retry_enabled: metadata.upstream_429_retry_enabled,
        upstream_429_max_retries: metadata.upstream_429_max_retries,
        available_models: Vec::new(),
        available_models_mode: None,
        available_models_defined: false,
        status_change_reasons: default_status_change_reasons(),
        timeouts: None,
    }
}

pub(crate) fn assign_current_forward_proxy(
    item: &mut UpstreamAccountSummary,
    state: &'static str,
    proxy_key: Option<String>,
    proxy_display_name: Option<String>,
) {
    item.current_forward_proxy_key = proxy_key;
    item.current_forward_proxy_display_name = proxy_display_name;
    item.current_forward_proxy_state = state.to_string();
}

pub(crate) fn resolve_current_forward_proxy_display_name(
    manager: &crate::forward_proxy::ForwardProxyManager,
    binding_display_names: &HashMap<String, String>,
    metadata_map: &HashMap<String, crate::forward_proxy::ForwardProxyMetadataHistoryRow>,
    binding_key: &str,
) -> Option<String> {
    binding_display_names.get(binding_key).cloned().or_else(|| {
        let history_row = metadata_map.get(binding_key);
        manager
            .resolve_current_or_historical_bound_proxy_key(binding_key, history_row)
            .and_then(|resolved_key| binding_display_names.get(&resolved_key).cloned())
            .or_else(|| {
                history_row
                    .map(|row| row.display_name.trim())
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
    })
}

pub(crate) async fn enrich_current_forward_proxy_for_summaries(
    state: &AppState,
    groups: &[UpstreamAccountGroupSummary],
    items: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }

    let group_map = groups
        .iter()
        .cloned()
        .map(|group| (group.group_name.clone(), group))
        .collect::<HashMap<_, _>>();
    let node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
    let (binding_display_names, shared_group_current_bindings, account_current_bindings) = {
        let manager = state.forward_proxy.lock().await;
        let binding_display_names = manager
            .binding_nodes()
            .into_iter()
            .map(|node| (node.key, node.display_name))
            .collect::<HashMap<_, _>>();
        let shared_group_current_bindings = groups
            .iter()
            .filter(|group| !group.node_shunt_enabled)
            .filter_map(|group| {
                manager
                    .current_bound_group_binding_key(&group.group_name, &group.bound_proxy_keys)
                    .map(|binding_key| (group.group_name.clone(), binding_key))
            })
            .collect::<HashMap<_, _>>();
        let account_current_bindings = items
            .iter()
            .filter(|item| !item.bound_proxy_keys.is_empty())
            .filter_map(|item| {
                manager
                    .current_bound_scope_binding_key(
                        &format!("account:{}", item.id),
                        &item.bound_proxy_keys,
                    )
                    .map(|binding_key| (item.id, binding_key))
            })
            .collect::<HashMap<_, _>>();
        (
            binding_display_names,
            shared_group_current_bindings,
            account_current_bindings,
        )
    };
    let relevant_binding_keys = shared_group_current_bindings
        .values()
        .cloned()
        .chain(account_current_bindings.values().cloned())
        .chain(node_shunt_assignments.account_proxy_keys.values().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let metadata_map = crate::forward_proxy::load_forward_proxy_metadata_history(
        &state.pool,
        &relevant_binding_keys,
    )
    .await?;
    let recovered_binding_display_names = {
        let manager = state.forward_proxy.lock().await;
        relevant_binding_keys
            .into_iter()
            .filter_map(|binding_key| {
                resolve_current_forward_proxy_display_name(
                    &manager,
                    &binding_display_names,
                    &metadata_map,
                    &binding_key,
                )
                .map(|display_name| (binding_key, display_name))
            })
            .collect::<HashMap<_, _>>()
    };

    for item in items {
        assign_current_forward_proxy(
            item,
            UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_UNCONFIGURED,
            None,
            None,
        );
        if let Some(binding_key) = account_current_bindings.get(&item.id).cloned() {
            assign_current_forward_proxy(
                item,
                UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED,
                Some(binding_key.clone()),
                recovered_binding_display_names.get(&binding_key).cloned(),
            );
            continue;
        }
        let Some(group_name) = normalize_optional_text(item.group_name.clone()) else {
            continue;
        };
        let Some(group) = group_map.get(&group_name) else {
            continue;
        };

        if group.node_shunt_enabled {
            if let Some(binding_key) = node_shunt_assignments.account_proxy_keys.get(&item.id) {
                assign_current_forward_proxy(
                    item,
                    UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED,
                    Some(binding_key.clone()),
                    recovered_binding_display_names.get(binding_key).cloned(),
                );
            } else if node_shunt_assignments
                .eligible_account_ids
                .contains(&item.id)
                && node_shunt_assignments
                    .group_slots
                    .get(&group_name)
                    .is_some_and(|slots| !slots.valid_proxy_keys.is_empty())
            {
                assign_current_forward_proxy(
                    item,
                    UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_PENDING,
                    None,
                    None,
                );
            }
            continue;
        }

        let Some(binding_key) = shared_group_current_bindings.get(&group_name).cloned() else {
            continue;
        };
        assign_current_forward_proxy(
            item,
            UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED,
            Some(binding_key.clone()),
            recovered_binding_display_names.get(&binding_key).cloned(),
        );
    }

    Ok(())
}

pub(crate) async fn enrich_current_forward_proxy_for_non_node_shunt_detail(
    state: &AppState,
    group: Option<&UpstreamAccountGroupSummary>,
    item: &mut UpstreamAccountSummary,
) -> Result<()> {
    assign_current_forward_proxy(
        item,
        UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_UNCONFIGURED,
        None,
        None,
    );
    if !item.bound_proxy_keys.is_empty() {
        let binding_key = {
            let manager = state.forward_proxy.lock().await;
            manager.current_bound_scope_binding_key(
                &format!("account:{}", item.id),
                &item.bound_proxy_keys,
            )
        };
        let Some(binding_key) = binding_key else {
            return Ok(());
        };
        let metadata_map = crate::forward_proxy::load_forward_proxy_metadata_history(
            &state.pool,
            std::slice::from_ref(&binding_key),
        )
        .await?;
        let binding_display_name = {
            let manager = state.forward_proxy.lock().await;
            let binding_display_names = manager
                .binding_nodes()
                .into_iter()
                .map(|node| (node.key, node.display_name))
                .collect::<HashMap<_, _>>();
            resolve_current_forward_proxy_display_name(
                &manager,
                &binding_display_names,
                &metadata_map,
                &binding_key,
            )
        };
        assign_current_forward_proxy(
            item,
            UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED,
            Some(binding_key),
            binding_display_name,
        );
        return Ok(());
    }
    let Some(group) = group else {
        return Ok(());
    };
    if group.node_shunt_enabled {
        return Ok(());
    }
    let binding_key = {
        let manager = state.forward_proxy.lock().await;
        manager.current_bound_group_binding_key(&group.group_name, &group.bound_proxy_keys)
    };
    let Some(binding_key) = binding_key else {
        return Ok(());
    };
    let metadata_map = crate::forward_proxy::load_forward_proxy_metadata_history(
        &state.pool,
        std::slice::from_ref(&binding_key),
    )
    .await?;
    let binding_display_name = {
        let manager = state.forward_proxy.lock().await;
        let binding_display_names = manager
            .binding_nodes()
            .into_iter()
            .map(|node| (node.key, node.display_name))
            .collect::<HashMap<_, _>>();
        resolve_current_forward_proxy_display_name(
            &manager,
            &binding_display_names,
            &metadata_map,
            &binding_key,
        )
    };
    assign_current_forward_proxy(
        item,
        UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED,
        Some(binding_key),
        binding_display_name,
    );
    Ok(())
}

pub(crate) fn collect_forward_proxy_catalog_keys(
    groups: &[UpstreamAccountGroupSummary],
    items: &[UpstreamAccountSummary],
) -> Vec<String> {
    let relevant_group_names = items
        .iter()
        .filter_map(|item| normalize_optional_text(item.group_name.clone()))
        .collect::<HashSet<_>>();
    groups
        .iter()
        .filter(|group| relevant_group_names.contains(&group.group_name))
        .flat_map(|group| group.bound_proxy_keys.iter().cloned())
        .chain(
            items
                .iter()
                .filter_map(|item| item.current_forward_proxy_key.clone()),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) async fn enrich_window_actual_usage_for_summaries(
    state: &AppState,
    items: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }

    let now = Utc::now();
    let Some((plans, query_start, query_end)) = collect_account_window_usage_plans(items, now)
    else {
        return Ok(());
    };
    let account_ids = plans.keys().copied().collect::<Vec<_>>();
    if account_ids.is_empty() {
        return Ok(());
    }

    let has_live_invocations = sqlite_table_exists(&state.pool, "codex_invocations").await?;
    let has_minute_stats_rollups =
        sqlite_table_exists(&state.pool, "upstream_account_stats_minute").await?;
    let has_hourly_usage_rollups =
        sqlite_table_exists(&state.pool, "upstream_account_usage_hourly").await?;
    if !has_live_invocations && !has_minute_stats_rollups && !has_hourly_usage_rollups {
        return Ok(());
    }

    let query_start_at = format_naive(query_start.with_timezone(&Shanghai).naive_local());
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let retention_cutoff_epoch = retention_cutoff.timestamp();
    let mut usage = account_ids
        .iter()
        .copied()
        .map(|account_id| (account_id, AccountWindowUsageSummary::default()))
        .collect::<HashMap<_, _>>();
    let minute_rows = if has_minute_stats_rollups {
        if let Some((full_minute_start_epoch, full_minute_end_epoch)) =
            collect_account_window_full_minute_bounds(&plans)
        {
            load_window_actual_usage_minute_rows_from_pool(
                &state.pool,
                &account_ids,
                full_minute_start_epoch,
                full_minute_end_epoch,
            )
            .await?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    if !minute_rows.is_empty() {
        fold_account_window_usage_minute_rows(&mut usage, &minute_rows, &plans);
    }
    let minute_covered_hourly_keys = minute_rows
        .iter()
        .map(|row| {
            (
                row.upstream_account_id,
                align_bucket_epoch(row.bucket_start_epoch, 3_600, 0),
            )
        })
        .collect::<HashSet<_>>();
    let mut covered_hourly_keys = minute_covered_hourly_keys.clone();
    if has_hourly_usage_rollups
        && let Some((full_hour_start_epoch, full_hour_end_epoch)) =
            collect_account_window_full_hour_bounds(&plans)
    {
        let hourly_rows = load_window_actual_usage_hourly_rows_from_pool(
            &state.pool,
            &account_ids,
            full_hour_start_epoch,
            full_hour_end_epoch,
        )
        .await?;
        let hourly_rows = hourly_rows
            .into_iter()
            .filter(|row| {
                !minute_covered_hourly_keys
                    .contains(&(row.upstream_account_id, row.bucket_start_epoch))
            })
            .collect::<Vec<_>>();
        covered_hourly_keys.extend(collect_account_window_hourly_coverage_keys(&hourly_rows));
        fold_account_window_usage_hourly_rows(&mut usage, &hourly_rows, &plans);
    }

    if has_live_invocations {
        let live_rollup_cursor =
            load_hourly_rollup_live_progress(&state.pool, HOURLY_ROLLUP_DATASET_INVOCATIONS)
                .await?;
        let partial_minute_bucket_epochs =
            collect_account_window_partial_minute_bucket_epochs(&plans)?;
        let partial_hour_bucket_epochs = collect_account_window_partial_bucket_epochs(&plans)?;
        let missing_full_hour_bucket_epochs =
            collect_account_window_missing_full_hour_bucket_epochs(&plans, &covered_hourly_keys);

        let archived_partial_minute_bucket_epochs = partial_minute_bucket_epochs
            .iter()
            .copied()
            .filter(|bucket_epoch| *bucket_epoch < retention_cutoff_epoch)
            .collect::<HashSet<_>>();
        let archived_partial_hour_bucket_epochs = partial_hour_bucket_epochs
            .iter()
            .copied()
            .filter(|bucket_epoch| {
                Utc.timestamp_opt(*bucket_epoch, 0)
                    .single()
                    .map(|bucket_start_at| bucket_start_at < retention_cutoff)
                    .unwrap_or(false)
            })
            .collect::<HashSet<_>>();
        let archived_missing_full_hour_bucket_epochs = missing_full_hour_bucket_epochs
            .iter()
            .copied()
            .filter(|bucket_epoch| *bucket_epoch < retention_cutoff_epoch)
            .collect::<HashSet<_>>();
        if query_start < retention_cutoff
            && (!archived_partial_minute_bucket_epochs.is_empty()
                || !archived_partial_hour_bucket_epochs.is_empty()
                || !archived_missing_full_hour_bucket_epochs.is_empty())
        {
            let archive_end = query_end.min(retention_cutoff - ChronoDuration::seconds(1));
            if query_start <= archive_end {
                let archive_end_at =
                    format_naive(archive_end.with_timezone(&Shanghai).naive_local());
                let archived_rows = load_window_actual_usage_rows_from_archives(
                    &state.pool,
                    &account_ids,
                    &query_start_at,
                    &archive_end_at,
                    &state.config.archive_dir,
                )
                .await?;
                let archived_rows = filter_account_window_usage_rows_for_exact_fallback(
                    archived_rows,
                    &archived_partial_minute_bucket_epochs,
                    &archived_partial_hour_bucket_epochs,
                    &archived_missing_full_hour_bucket_epochs,
                    &covered_hourly_keys,
                )?;
                for (account_id, summary) in fold_account_window_usage_rows(archived_rows, &plans) {
                    usage.entry(account_id).or_default().merge(summary);
                }
            }
        }

        let live_partial_minute_bucket_epochs = partial_minute_bucket_epochs
            .iter()
            .copied()
            .filter(|bucket_epoch| *bucket_epoch >= retention_cutoff_epoch)
            .collect::<HashSet<_>>();
        let live_partial_hour_bucket_epochs = partial_hour_bucket_epochs
            .iter()
            .copied()
            .filter(|bucket_epoch| *bucket_epoch >= retention_cutoff_epoch)
            .chain(
                missing_full_hour_bucket_epochs
                    .iter()
                    .copied()
                    .filter(|bucket_epoch| *bucket_epoch >= retention_cutoff_epoch),
            )
            .collect::<HashSet<_>>();
        let live_missing_full_hour_bucket_epochs = missing_full_hour_bucket_epochs
            .iter()
            .copied()
            .filter(|bucket_epoch| *bucket_epoch >= retention_cutoff_epoch)
            .collect::<HashSet<_>>();
        let live_exact_fallback_bucket_epochs = live_partial_hour_bucket_epochs
            .iter()
            .copied()
            .chain(live_partial_minute_bucket_epochs.iter().copied())
            .chain(live_missing_full_hour_bucket_epochs.iter().copied())
            .collect::<HashSet<_>>();
        if !live_exact_fallback_bucket_epochs.is_empty() {
            let live_rows = load_window_actual_usage_rows_from_pool(
                &state.pool,
                &account_ids,
                &query_start_at,
                &format_naive(query_end.with_timezone(&Shanghai).naive_local()),
                None,
                None,
                (live_rollup_cursor > 0).then_some(live_rollup_cursor),
            )
            .await?;
            let live_rows = filter_account_window_usage_rows_for_exact_fallback(
                live_rows,
                &live_partial_minute_bucket_epochs,
                &live_partial_hour_bucket_epochs,
                &live_missing_full_hour_bucket_epochs,
                &covered_hourly_keys,
            )?;
            for (account_id, summary) in fold_account_window_usage_rows(live_rows, &plans) {
                usage.entry(account_id).or_default().merge(summary);
            }
        }

        if live_rollup_cursor > 0 {
            let query_end_at = format_naive(query_end.with_timezone(&Shanghai).naive_local());
            let live_tail_rows = load_window_actual_usage_rows_from_pool(
                &state.pool,
                &account_ids,
                &query_start_at,
                &query_end_at,
                None,
                Some(live_rollup_cursor),
                None,
            )
            .await?;
            for (account_id, summary) in fold_account_window_usage_rows(live_tail_rows, &plans) {
                usage.entry(account_id).or_default().merge(summary);
            }
        }
    }

    apply_window_actual_usage_to_summaries(items, &usage);
    Ok(())
}
