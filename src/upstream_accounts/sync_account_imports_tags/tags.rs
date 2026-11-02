use super::*;
pub(crate) async fn load_account_tag_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, Vec<AccountTagSummary>>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            link.account_id,
            tag.id AS tag_id,
            tag.name,
            tag.system_key,
            tag.protected,
            tag.allow_cut_out,
            tag.allow_cut_in,
            tag.priority_tier,
            tag.fast_mode_rewrite_mode,
            tag.concurrency_limit,
            tag.upstream_429_retry_enabled,
            tag.upstream_429_max_retries,
            tag.available_models_json
        FROM pool_upstream_account_tags link
        INNER JOIN pool_tags tag ON tag.id = link.tag_id
        WHERE link.account_id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    let rows = query
        .push(") ORDER BY tag.name COLLATE NOCASE ASC, tag.id ASC")
        .build_query_as::<AccountTagRow>()
        .fetch_all(pool)
        .await?;
    let mut grouped: HashMap<i64, Vec<AccountTagSummary>> = HashMap::new();
    for row in rows {
        grouped
            .entry(row.account_id)
            .or_default()
            .push(account_tag_summary_from_row(&row));
    }
    Ok(grouped)
}

pub(crate) async fn load_tags_by_ids(pool: &Pool<Sqlite>, tag_ids: &[i64]) -> Result<Vec<TagRow>> {
    if tag_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            name,
            system_key,
            protected,
            allow_cut_out,
            allow_cut_in,
            priority_tier,
            fast_mode_rewrite_mode,
            concurrency_limit,
            upstream_429_retry_enabled,
            upstream_429_max_retries
        FROM pool_tags
        WHERE id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for tag_id in tag_ids {
            separated.push_bind(tag_id);
        }
    }
    query
        .push(") ORDER BY name COLLATE NOCASE ASC, id ASC")
        .build_query_as::<TagRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_tag_row(pool: &Pool<Sqlite>, tag_id: i64) -> Result<Option<TagRow>> {
    sqlx::query_as::<_, TagRow>(
        r#"
        SELECT
            name,
            system_key,
            protected,
            allow_cut_out,
            allow_cut_in,
            priority_tier,
            fast_mode_rewrite_mode,
            concurrency_limit,
            upstream_429_retry_enabled,
            upstream_429_max_retries
        FROM pool_tags
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(tag_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_tag_detail(pool: &Pool<Sqlite>, tag_id: i64) -> Result<Option<TagDetail>> {
    let items = load_tag_summaries(
        pool,
        &ListTagsQuery {
            search: None,
            has_accounts: None,
            allow_cut_in: None,
            allow_cut_out: None,
        },
    )
    .await?;
    Ok(items
        .into_iter()
        .find(|item| item.id == tag_id)
        .map(|summary| TagDetail { summary }))
}

pub(crate) async fn load_tag_summaries(
    pool: &Pool<Sqlite>,
    params: &ListTagsQuery,
) -> Result<Vec<TagSummary>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            tag.id,
            tag.name,
            tag.system_key,
            tag.protected,
            tag.allow_cut_out,
            tag.allow_cut_in,
            tag.priority_tier,
            tag.fast_mode_rewrite_mode,
            tag.concurrency_limit,
            tag.upstream_429_retry_enabled,
            tag.upstream_429_max_retries,
            tag.available_models_json,
            tag.updated_at,
            COUNT(DISTINCT link.account_id) AS account_count,
            COUNT(DISTINCT NULLIF(TRIM(account.group_name), '')) AS group_count
        FROM pool_tags tag
        LEFT JOIN pool_upstream_account_tags link ON link.tag_id = tag.id
        LEFT JOIN pool_upstream_accounts account ON account.id = link.account_id
        WHERE tag.system_key IS NOT NULL
        "#,
    );
    if let Some(search) = params
        .search
        .as_ref()
        .and_then(|value| normalize_optional_text(Some(value.clone())))
    {
        query
            .push(" AND tag.name LIKE ")
            .push_bind(format!("%{search}%"));
    }
    if let Some(allow_cut_in) = params.allow_cut_in {
        query
            .push(" AND tag.allow_cut_in = ")
            .push_bind(if allow_cut_in { 1 } else { 0 });
    }
    if let Some(allow_cut_out) = params.allow_cut_out {
        query
            .push(" AND tag.allow_cut_out = ")
            .push_bind(if allow_cut_out { 1 } else { 0 });
    }
    query.push(
        " GROUP BY tag.id, tag.name, tag.system_key, tag.protected, tag.allow_cut_out, tag.allow_cut_in, tag.priority_tier, tag.fast_mode_rewrite_mode, tag.concurrency_limit, tag.upstream_429_retry_enabled, tag.upstream_429_max_retries, tag.available_models_json, tag.updated_at",
    );
    if let Some(has_accounts) = params.has_accounts {
        query.push(if has_accounts {
            " HAVING COUNT(DISTINCT link.account_id) > 0"
        } else {
            " HAVING COUNT(DISTINCT link.account_id) = 0"
        });
    }
    let rows = query
        .push(" ORDER BY tag.updated_at DESC, tag.id DESC")
        .build_query_as::<TagListRow>()
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| tag_summary_from_row(&row))
        .collect())
}

pub(crate) async fn insert_tag(
    pool: &Pool<Sqlite>,
    name: &str,
    rule: &TagRoutingRule,
) -> Result<TagDetail> {
    let now_iso = format_utc_iso(Utc::now());
    let inserted_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_tags (
            name, allow_cut_out, allow_cut_in, priority_tier, fast_mode_rewrite_mode,
            concurrency_limit, upstream_429_retry_enabled, upstream_429_max_retries,
            available_models_json, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
        RETURNING id
        "#,
    )
    .bind(name)
    .bind(if rule.allow_cut_out { 1 } else { 0 })
    .bind(if rule.allow_cut_in { 1 } else { 0 })
    .bind(rule.priority_tier.as_str())
    .bind(rule.fast_mode_rewrite_mode.as_str())
    .bind(rule.concurrency_limit)
    .bind(if rule.upstream_429_retry_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(i64::from(rule.upstream_429_max_retries))
    .bind(encode_string_array_json(&rule.available_models)?)
    .bind(&now_iso)
    .fetch_one(pool)
    .await?;
    load_tag_detail(pool, inserted_id)
        .await?
        .ok_or_else(|| anyhow!("tag not found after insert"))
}

pub(crate) async fn persist_tag_update(
    pool: &Pool<Sqlite>,
    tag_id: i64,
    name: &str,
    rule: &TagRoutingRule,
) -> Result<TagDetail> {
    let now_iso = format_utc_iso(Utc::now());
    let affected = sqlx::query(
        r#"
        UPDATE pool_tags
        SET name = ?2,
            allow_cut_out = ?3,
            allow_cut_in = ?4,
            priority_tier = ?5,
            fast_mode_rewrite_mode = ?6,
            concurrency_limit = ?7,
            upstream_429_retry_enabled = ?8,
            upstream_429_max_retries = ?9,
            available_models_json = ?10,
            updated_at = ?11
        WHERE id = ?1
          AND protected = 0
          AND system_key IS NULL
        "#,
    )
    .bind(tag_id)
    .bind(name)
    .bind(if rule.allow_cut_out { 1 } else { 0 })
    .bind(if rule.allow_cut_in { 1 } else { 0 })
    .bind(rule.priority_tier.as_str())
    .bind(rule.fast_mode_rewrite_mode.as_str())
    .bind(rule.concurrency_limit)
    .bind(if rule.upstream_429_retry_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(i64::from(rule.upstream_429_max_retries))
    .bind(encode_string_array_json(&rule.available_models)?)
    .bind(&now_iso)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(anyhow!("system tag cannot be edited"));
    }
    load_tag_detail(pool, tag_id)
        .await?
        .ok_or_else(|| anyhow!("tag not found after update"))
}

pub(crate) async fn delete_tag_by_id(
    pool: &Pool<Sqlite>,
    tag_id: i64,
) -> Result<(), (StatusCode, String)> {
    let is_protected = sqlx::query_scalar::<_, i64>(
            "SELECT CASE WHEN protected != 0 OR system_key IS NOT NULL THEN 1 ELSE 0 END FROM pool_tags WHERE id = ?1",
        )
            .bind(tag_id)
            .fetch_optional(pool)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    if is_protected != 0 {
        return Err((
            StatusCode::CONFLICT,
            "system tag cannot be deleted".to_string(),
        ));
    }

    let linked_account_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pool_upstream_account_tags WHERE tag_id = ?1",
    )
    .bind(tag_id)
    .fetch_one(pool)
    .await
    .map_err(internal_error_tuple)?;
    let linked_session_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_oauth_login_sessions
        WHERE tag_ids_json IS NOT NULL
          AND EXISTS (
              SELECT 1
              FROM json_each(pool_oauth_login_sessions.tag_ids_json)
              WHERE CAST(json_each.value AS INTEGER) = ?1
          )
        "#,
    )
    .bind(tag_id)
    .fetch_one(pool)
    .await
    .map_err(internal_error_tuple)?;
    if linked_account_count > 0 || linked_session_count > 0 {
        return Err((
            StatusCode::CONFLICT,
            "tag is still associated with accounts or pending OAuth sessions".to_string(),
        ));
    }
    let affected =
        sqlx::query("DELETE FROM pool_tags WHERE id = ?1 AND protected = 0 AND system_key IS NULL")
            .bind(tag_id)
            .execute(pool)
            .await
            .map_err(internal_error_tuple)?
            .rows_affected();
    if affected == 0 {
        let still_exists =
            sqlx::query_scalar::<_, i64>("SELECT 1 FROM pool_tags WHERE id = ?1 LIMIT 1")
                .bind(tag_id)
                .fetch_optional(pool)
                .await
                .map_err(internal_error_tuple)?
                .is_some();
        return Err(if still_exists {
            (
                StatusCode::CONFLICT,
                "system tag cannot be deleted".to_string(),
            )
        } else {
            (StatusCode::NOT_FOUND, "tag not found".to_string())
        });
    }
    Ok(())
}

pub(crate) fn map_tag_write_error(err: anyhow::Error) -> (StatusCode, String) {
    let message = err.to_string();
    if message.contains("system tag cannot be edited") {
        (
            StatusCode::CONFLICT,
            "system tag cannot be edited".to_string(),
        )
    } else if message.contains("UNIQUE constraint failed") {
        (StatusCode::CONFLICT, "tag name already exists".to_string())
    } else {
        internal_error_tuple(err)
    }
}

pub(crate) async fn validate_tag_ids(
    pool: &Pool<Sqlite>,
    tag_ids: &[i64],
) -> Result<Vec<i64>, (StatusCode, String)> {
    let mut normalized = tag_ids
        .iter()
        .copied()
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.is_empty() {
        return Ok(normalized);
    }
    let rows = load_tags_by_ids(pool, &normalized)
        .await
        .map_err(internal_error_tuple)?;
    if rows.len() != normalized.len() {
        return Err((
            StatusCode::BAD_REQUEST,
            "one or more tagIds do not exist".to_string(),
        ));
    }
    Ok(normalized)
}

pub(crate) async fn sync_account_tag_links_with_executor(
    conn: &mut SqliteConnection,
    account_id: i64,
    tag_ids: &[i64],
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query("DELETE FROM pool_upstream_account_tags WHERE account_id = ?1")
        .bind(account_id)
        .execute(&mut *conn)
        .await?;
    for tag_id in tag_ids {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_tags (
                account_id, tag_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?3)
            "#,
        )
        .bind(account_id)
        .bind(tag_id)
        .bind(&now_iso)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

pub(crate) async fn sync_account_tag_links(
    pool: &Pool<Sqlite>,
    account_id: i64,
    tag_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sync_account_tag_links_with_executor(&mut tx, account_id, tag_ids).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn load_upstream_account_groups(
    state: &AppState,
) -> Result<Vec<UpstreamAccountGroupSummary>> {
    let rows = sqlx::query_as::<_, UpstreamAccountGroupListRow>(
        r#"
        WITH account_groups AS (
            SELECT
                TRIM(group_name) AS group_name,
                COUNT(*) AS account_count
            FROM pool_upstream_accounts
            WHERE deleted_at IS NULL
              AND kind = 'oauth_codex'
              AND group_name IS NOT NULL AND TRIM(group_name) <> ''
            GROUP BY TRIM(group_name)
        ),
        catalog_groups AS (
            SELECT group_name FROM account_groups
            UNION
            SELECT DISTINCT TRIM(group_name) AS group_name
            FROM pool_upstream_account_group_notes
            WHERE TRIM(group_name) <> ''
        )
        SELECT
            catalog_groups.group_name,
            COALESCE(account_groups.account_count, 0) AS account_count,
            notes.note,
            notes.bound_proxy_keys_json,
            notes.node_shunt_enabled,
            notes.single_account_rotation_enabled,
            notes.upstream_429_retry_enabled,
            notes.upstream_429_max_retries,
            notes.concurrency_limit,
            notes.policy_allow_cut_out,
            notes.policy_allow_cut_in,
            notes.policy_priority_tier,
            notes.policy_fast_mode_rewrite_mode,
            notes.policy_image_tool_rewrite_mode,
            notes.policy_codex_imagegen_rewrite_mode,
            notes.policy_request_compression_algorithm,
            notes.policy_oauth_request_compression_algorithm,
            notes.policy_concurrency_limit,
            notes.policy_upstream_429_retry_enabled,
            notes.policy_upstream_429_max_retries,
            notes.policy_available_models_json,
            notes.policy_available_models_mode,
            notes.policy_status_change_upstream_http_401,
            notes.policy_status_change_upstream_http_402,
            notes.policy_status_change_upstream_http_403,
            notes.policy_status_change_reauth_required,
            notes.policy_status_change_upstream_http_429_rate_limit,
            notes.policy_status_change_upstream_http_429_quota_exhausted,
            notes.policy_status_change_usage_snapshot_exhausted,
            notes.policy_status_change_quota_still_exhausted,
            notes.policy_status_change_transport_failure,
            notes.policy_status_change_upstream_server_overloaded,
            notes.policy_status_change_upstream_http_5xx,
            notes.policy_responses_first_byte_timeout_secs,
            notes.policy_compact_first_byte_timeout_secs,
            notes.policy_image_first_byte_timeout_secs,
            notes.policy_responses_stream_timeout_secs,
            notes.policy_compact_stream_timeout_secs
        FROM catalog_groups
        LEFT JOIN account_groups
            ON account_groups.group_name = catalog_groups.group_name
        LEFT JOIN pool_upstream_account_group_notes notes
            ON notes.group_name = catalog_groups.group_name
        ORDER BY catalog_groups.group_name COLLATE NOCASE ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await?;

    let mut groups = Vec::with_capacity(rows.len());
    for row in rows {
        let node_shunt_enabled =
            decode_group_node_shunt_enabled(row.node_shunt_enabled.unwrap_or_default());
        let single_account_rotation_enabled = decode_group_single_account_rotation_enabled(
            row.single_account_rotation_enabled.unwrap_or_default(),
        );
        let upstream_429_retry_enabled = decode_group_upstream_429_retry_enabled(
            row.upstream_429_retry_enabled.unwrap_or_default(),
        );
        let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
            upstream_429_retry_enabled,
            decode_group_upstream_429_max_retries(row.upstream_429_max_retries.unwrap_or_default()),
        );
        let routing_rule = group_routing_rule_from_columns(
            row.concurrency_limit.unwrap_or_default(),
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            row.policy_allow_cut_out,
            row.policy_allow_cut_in,
            row.policy_priority_tier.as_deref(),
            row.policy_fast_mode_rewrite_mode.as_deref(),
            row.policy_image_tool_rewrite_mode.as_deref(),
            row.policy_codex_imagegen_rewrite_mode.as_deref(),
            row.policy_request_compression_algorithm.as_deref(),
            row.policy_oauth_request_compression_algorithm.as_deref(),
            row.policy_concurrency_limit,
            row.policy_upstream_429_retry_enabled,
            row.policy_upstream_429_max_retries,
            row.policy_available_models_json.as_deref(),
            row.policy_available_models_mode.as_deref(),
            row.policy_status_change_upstream_http_401,
            row.policy_status_change_upstream_http_402,
            row.policy_status_change_upstream_http_403,
            row.policy_status_change_reauth_required,
            row.policy_status_change_upstream_http_429_rate_limit,
            row.policy_status_change_upstream_http_429_quota_exhausted,
            row.policy_status_change_usage_snapshot_exhausted,
            row.policy_status_change_quota_still_exhausted,
            row.policy_status_change_transport_failure,
            row.policy_status_change_upstream_server_overloaded,
            row.policy_status_change_upstream_http_5xx,
            row.policy_responses_first_byte_timeout_secs,
            row.policy_compact_first_byte_timeout_secs,
            row.policy_image_first_byte_timeout_secs,
            row.policy_responses_stream_timeout_secs,
            row.policy_compact_stream_timeout_secs,
        );
        let (effective_timeouts, timeout_field_sources, _) =
            load_effective_request_path_timeouts_for_group(
                &state.pool,
                &state.config,
                Some(row.group_name.as_str()),
            )
            .await?;
        groups.push(UpstreamAccountGroupSummary {
            group_name: row.group_name,
            account_count: row.account_count,
            note: normalize_optional_text(row.note),
            bound_proxy_keys: decode_group_bound_proxy_keys_json(
                row.bound_proxy_keys_json.as_deref(),
            ),
            node_shunt_enabled,
            single_account_rotation_enabled,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            concurrency_limit: row.concurrency_limit.unwrap_or_default(),
            effective_timeouts,
            timeout_field_sources,
            routing_rule,
        });
    }
    Ok(groups)
}
