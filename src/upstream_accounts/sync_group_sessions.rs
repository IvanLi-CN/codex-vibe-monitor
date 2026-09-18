include!("sync_group_sessions/part_01.rs");
include!("sync_group_sessions/part_03.rs");
include!("sync_group_sessions/part_04.rs");
pub(crate) async fn backfill_upstream_account_last_activity_from_live_invocations(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    if !sqlite_table_exists(pool, "codex_invocations")
        .await
        .context("failed to inspect codex_invocations existence")?
    {
        return Ok(0);
    }

    let updated = sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET last_activity_at = (
                SELECT MAX(occurred_at)
                FROM codex_invocations
                WHERE CASE
                    WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER)
                END = pool_upstream_accounts.id
            ),
            last_activity_live_backfill_completed = 1
        WHERE COALESCE(deleted_at, '') = ''
          AND last_activity_at IS NULL
          AND last_activity_live_backfill_completed = 0
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill pool_upstream_accounts.last_activity_at from live invocations")?;
    Ok(updated.rows_affected())
}

pub(crate) async fn group_has_accounts(pool: &Pool<Sqlite>, group_name: &str) -> Result<bool> {
    let mut conn = pool.acquire().await?;
    group_has_accounts_conn(&mut conn, group_name).await
}

pub(crate) async fn group_account_count_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE COALESCE(deleted_at, '') = ''
          AND kind = ?2
          AND group_name = ?1
        "#,
    )
    .bind(group_name)
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .fetch_one(conn)
    .await
    .map_err(Into::into)
}

pub(crate) async fn group_has_accounts_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
) -> Result<bool> {
    Ok(group_account_count_conn(conn, group_name).await? > 0)
}

pub(crate) fn normalize_bound_proxy_keys(bound_proxy_keys: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    bound_proxy_keys
        .into_iter()
        .filter_map(|value| normalize_optional_text(Some(value)))
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub(crate) fn decode_group_bound_proxy_keys_json(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .map(normalize_bound_proxy_keys)
        .unwrap_or_default()
}

pub(crate) fn decode_group_node_shunt_enabled(raw: i64) -> bool {
    raw != 0
}

pub(crate) fn decode_group_single_account_rotation_enabled(raw: i64) -> bool {
    raw != 0
}

pub(crate) fn decode_group_requested_flag(raw: i64) -> bool {
    raw != 0
}

pub(crate) fn decode_group_upstream_429_retry_enabled(raw: i64) -> bool {
    raw != 0
}

pub(crate) fn normalize_group_upstream_429_max_retries(value: u8) -> u8 {
    value.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES)
}

pub(crate) fn normalize_enabled_group_upstream_429_max_retries(value: u8) -> u8 {
    normalize_group_upstream_429_max_retries(value).max(1)
}

pub(crate) fn normalize_group_upstream_429_retry_metadata(
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
) -> u8 {
    if upstream_429_retry_enabled {
        normalize_enabled_group_upstream_429_max_retries(upstream_429_max_retries)
    } else {
        0
    }
}

pub(crate) fn decode_group_upstream_429_max_retries(raw: i64) -> u8 {
    normalize_group_upstream_429_max_retries(raw.max(0) as u8)
}

pub(crate) fn encode_group_bound_proxy_keys_json(bound_proxy_keys: &[String]) -> Result<String> {
    serde_json::to_string(bound_proxy_keys).context("failed to encode group bound proxy keys")
}

pub(crate) fn group_node_shunt_unassigned_error_message() -> &'static str {
    UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED_MESSAGE
}

pub(crate) fn group_node_shunt_unassigned_error() -> anyhow::Error {
    anyhow!(group_node_shunt_unassigned_error_message())
}

pub(crate) fn is_group_node_shunt_unassigned_message(message: &str) -> bool {
    message
        .trim()
        .contains(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED_MESSAGE)
}

pub(crate) fn missing_request_group_error_message() -> String {
    "groupName is required for upstream accounts".to_string()
}

pub(crate) fn missing_account_group_error_message() -> String {
    "upstream account is not assigned to a group; assign it to a group with at least one bound forward proxy node".to_string()
}

pub(crate) fn missing_group_bound_proxy_error_message(group_name: &str) -> String {
    format!(
        "upstream account group \"{group_name}\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    )
}

pub(crate) fn missing_selectable_group_bound_proxy_error_message(group_name: &str) -> String {
    format!("upstream account group \"{group_name}\" has no selectable bound forward proxy nodes")
}

#[derive(Default)]
pub(crate) struct RequestedGroupMetadataInput {
    pub(crate) note: Option<String>,
    pub(crate) note_was_requested: bool,
    pub(crate) bound_proxy_keys: Option<Vec<String>>,
    pub(crate) bound_proxy_keys_was_requested: bool,
    pub(crate) concurrency_limit: i64,
    pub(crate) concurrency_limit_was_requested: bool,
    pub(crate) node_shunt_enabled: Option<bool>,
    pub(crate) node_shunt_enabled_was_requested: bool,
    pub(crate) single_account_rotation_enabled: Option<bool>,
    pub(crate) single_account_rotation_enabled_was_requested: bool,
}

impl RequestedGroupMetadataInput {
    pub(crate) fn from_external_metadata(
        metadata: &ExternalUpstreamAccountMetadataRequest,
        concurrency_limit: i64,
    ) -> Self {
        Self::default()
            .with_note(
                normalize_optional_text(metadata.group_note.clone()),
                metadata.group_note.is_some(),
            )
            .with_bound_proxy_keys(
                metadata.group_bound_proxy_keys.clone(),
                metadata.group_bound_proxy_keys.is_some(),
            )
            .with_concurrency_limit(concurrency_limit, metadata.concurrency_limit.is_some())
            .with_node_shunt(
                metadata.group_node_shunt_enabled,
                metadata.group_node_shunt_enabled.is_some(),
            )
            .with_single_account_rotation(
                metadata.group_single_account_rotation_enabled,
                metadata.group_single_account_rotation_enabled.is_some(),
            )
    }

    pub(crate) fn from_import_values(
        note: Option<String>,
        bound_proxy_keys: Option<Vec<String>>,
        concurrency_limit: i64,
        concurrency_limit_was_requested: bool,
        node_shunt_enabled: Option<bool>,
        single_account_rotation_enabled: Option<bool>,
    ) -> Self {
        Self::default()
            .with_note(note.clone(), note.is_some())
            .with_bound_proxy_keys(bound_proxy_keys.clone(), bound_proxy_keys.is_some())
            .with_concurrency_limit(concurrency_limit, concurrency_limit_was_requested)
            .with_node_shunt(node_shunt_enabled, node_shunt_enabled.is_some())
            .with_single_account_rotation(
                single_account_rotation_enabled,
                single_account_rotation_enabled.is_some(),
            )
    }

    pub(crate) fn from_update_request(
        payload: &UpdateUpstreamAccountRequest,
        note: Option<String>,
        concurrency_limit: i64,
    ) -> Self {
        Self::default()
            .with_note(note, payload.group_note.is_some())
            .with_bound_proxy_keys(
                payload.group_bound_proxy_keys.clone(),
                payload.group_bound_proxy_keys.is_some(),
            )
            .with_concurrency_limit(concurrency_limit, payload.concurrency_limit.is_some())
            .with_node_shunt(
                payload.group_node_shunt_enabled,
                payload.group_node_shunt_enabled.is_some(),
            )
            .with_single_account_rotation(
                payload.group_single_account_rotation_enabled,
                payload.group_single_account_rotation_enabled.is_some(),
            )
    }

    pub(crate) fn from_oauth_login_session(session: &OauthLoginSessionRow) -> Self {
        Self::default()
            .with_note(session.group_note.clone(), true)
            .with_bound_proxy_keys(
                Some(decode_group_bound_proxy_keys_json(
                    session.group_bound_proxy_keys_json.as_deref(),
                )),
                true,
            )
            .with_concurrency_limit(session.group_concurrency_limit, true)
            .with_node_shunt(
                Some(decode_group_node_shunt_enabled(
                    session.group_node_shunt_enabled,
                )),
                decode_group_requested_flag(session.group_node_shunt_enabled_requested),
            )
            .with_single_account_rotation(
                Some(decode_group_single_account_rotation_enabled(
                    session.group_single_account_rotation_enabled,
                )),
                decode_group_requested_flag(
                    session.group_single_account_rotation_enabled_requested,
                ),
            )
    }

    pub(crate) fn from_oauth_login_update_parts(
        note: (Option<String>, bool),
        binding: &ResolvedRequiredGroupProxyBinding,
        bound_proxy_and_node_requested: (bool, bool),
        concurrency: (i64, bool),
        single_account_rotation: (Option<bool>, bool),
    ) -> Self {
        Self::default()
            .with_note(note.0, note.1)
            .with_bound_proxy_keys(
                Some(binding.bound_proxy_keys.clone()),
                bound_proxy_and_node_requested.0,
            )
            .with_concurrency_limit(concurrency.0, concurrency.1)
            .with_node_shunt(
                Some(binding.node_shunt_enabled),
                bound_proxy_and_node_requested.1,
            )
            .with_single_account_rotation(single_account_rotation.0, single_account_rotation.1)
    }

    pub(crate) fn with_note(mut self, note: Option<String>, requested: bool) -> Self {
        self.note = note;
        self.note_was_requested = requested;
        self
    }

    pub(crate) fn with_bound_proxy_keys(
        mut self,
        bound_proxy_keys: Option<Vec<String>>,
        requested: bool,
    ) -> Self {
        self.bound_proxy_keys = bound_proxy_keys;
        self.bound_proxy_keys_was_requested = requested;
        self
    }

    pub(crate) fn with_concurrency_limit(mut self, limit: i64, requested: bool) -> Self {
        self.concurrency_limit = limit;
        self.concurrency_limit_was_requested = requested;
        self
    }

    pub(crate) fn with_node_shunt(mut self, enabled: Option<bool>, requested: bool) -> Self {
        self.node_shunt_enabled = enabled;
        self.node_shunt_enabled_was_requested = requested;
        self
    }

    pub(crate) fn with_single_account_rotation(
        mut self,
        enabled: Option<bool>,
        requested: bool,
    ) -> Self {
        self.single_account_rotation_enabled = enabled;
        self.single_account_rotation_enabled_was_requested = requested;
        self
    }
}

pub(crate) fn build_requested_group_metadata_changes(
    input: RequestedGroupMetadataInput,
) -> RequestedGroupMetadataChanges {
    let RequestedGroupMetadataInput {
        note,
        note_was_requested,
        bound_proxy_keys,
        bound_proxy_keys_was_requested,
        concurrency_limit,
        concurrency_limit_was_requested,
        node_shunt_enabled,
        node_shunt_enabled_was_requested,
        single_account_rotation_enabled,
        single_account_rotation_enabled_was_requested,
    } = input;
    RequestedGroupMetadataChanges {
        note: normalize_optional_text(note),
        note_was_requested,
        bound_proxy_keys: if bound_proxy_keys_was_requested {
            normalize_bound_proxy_keys(bound_proxy_keys.unwrap_or_default())
        } else {
            Vec::new()
        },
        bound_proxy_keys_was_requested,
        concurrency_limit,
        concurrency_limit_was_requested,
        node_shunt_enabled: node_shunt_enabled.unwrap_or(false),
        node_shunt_enabled_was_requested,
        single_account_rotation_enabled: single_account_rotation_enabled.unwrap_or(false),
        single_account_rotation_enabled_was_requested,
    }
}

pub(crate) fn required_account_forward_proxy_scope(
    group_name: Option<&str>,
    bound_proxy_keys: Vec<String>,
) -> Result<ForwardProxyRouteScope> {
    let normalized_group_name = group_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!(missing_account_group_error_message()))?;
    let normalized_bound_proxy_keys = normalize_bound_proxy_keys(bound_proxy_keys);
    if normalized_bound_proxy_keys.is_empty() {
        bail!(missing_group_bound_proxy_error_message(
            &normalized_group_name
        ));
    }
    Ok(ForwardProxyRouteScope::BoundGroup {
        group_name: normalized_group_name,
        bound_proxy_keys: normalized_bound_proxy_keys,
    })
}

pub(crate) fn map_required_group_proxy_selection_error(
    scope: &ForwardProxyRouteScope,
    err: anyhow::Error,
) -> anyhow::Error {
    match scope {
        ForwardProxyRouteScope::BoundGroup { group_name, .. }
            if err
                .to_string()
                .contains("bound forward proxy group has no selectable nodes") =>
        {
            anyhow!(missing_selectable_group_bound_proxy_error_message(
                group_name
            ))
        }
        _ => err,
    }
}

pub(crate) async fn ensure_required_group_proxy_scope_selectable(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<()> {
    select_forward_proxy_for_scope(state, scope)
        .await
        .map(|_| ())
        .map_err(|err| map_required_group_proxy_selection_error(scope, err))
}

pub(crate) async fn resolve_required_group_proxy_binding_for_write(
    state: &AppState,
    group_name: Option<String>,
    requested_bound_proxy_keys: Option<Vec<String>>,
    requested_node_shunt_enabled: Option<bool>,
) -> Result<ResolvedRequiredGroupProxyBinding, (StatusCode, String)> {
    let group_name = normalize_optional_text(group_name).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            missing_request_group_error_message(),
        )
    })?;
    let existing_metadata = load_group_metadata(&state.pool, Some(&group_name))
        .await
        .map_err(internal_error_tuple)?;
    let bound_proxy_keys = if let Some(requested_bound_proxy_keys) = requested_bound_proxy_keys {
        normalize_bound_proxy_keys(requested_bound_proxy_keys)
    } else {
        existing_metadata.bound_proxy_keys.clone()
    };
    let bound_proxy_keys = canonicalize_forward_proxy_bound_keys(state, &bound_proxy_keys)
        .await
        .map_err(internal_error_tuple)?;
    let node_shunt_enabled =
        requested_node_shunt_enabled.unwrap_or(existing_metadata.node_shunt_enabled);
    if bound_proxy_keys.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            missing_group_bound_proxy_error_message(&group_name),
        ));
    }
    if !node_shunt_enabled {
        let scope =
            required_account_forward_proxy_scope(Some(&group_name), bound_proxy_keys.clone())
                .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
        ensure_required_group_proxy_scope_selectable(state, &scope)
            .await
            .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    }
    Ok(ResolvedRequiredGroupProxyBinding {
        group_name,
        bound_proxy_keys,
        node_shunt_enabled,
    })
}

pub(crate) async fn load_group_metadata_conn(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    group_name: &str,
) -> Result<Option<UpstreamAccountGroupMetadata>> {
    sqlx::query_as::<_, (String, Option<String>, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            note,
            bound_proxy_keys_json,
            node_shunt_enabled,
            single_account_rotation_enabled,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            concurrency_limit
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind(group_name)
    .fetch_optional(executor)
    .await
    .map(|row| {
        row.map(
            |(
                note,
                bound_proxy_keys_json,
                node_shunt_enabled,
                single_account_rotation_enabled,
                upstream_429_retry_enabled,
                upstream_429_max_retries,
                concurrency_limit,
            )| {
                let node_shunt_enabled = decode_group_node_shunt_enabled(node_shunt_enabled);
                let single_account_rotation_enabled =
                    decode_group_single_account_rotation_enabled(single_account_rotation_enabled);
                let upstream_429_retry_enabled =
                    decode_group_upstream_429_retry_enabled(upstream_429_retry_enabled);
                let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
                    upstream_429_retry_enabled,
                    decode_group_upstream_429_max_retries(upstream_429_max_retries),
                );
                UpstreamAccountGroupMetadata {
                    note: normalize_optional_text(Some(note)),
                    bound_proxy_keys: decode_group_bound_proxy_keys_json(
                        bound_proxy_keys_json.as_deref(),
                    ),
                    node_shunt_enabled,
                    single_account_rotation_enabled,
                    upstream_429_retry_enabled,
                    upstream_429_max_retries,
                    concurrency_limit,
                }
            },
        )
    })
    .map_err(Into::into)
}

pub(crate) async fn load_group_metadata(
    pool: &Pool<Sqlite>,
    group_name: Option<&str>,
) -> Result<UpstreamAccountGroupMetadata> {
    let Some(group_name) = group_name else {
        return Ok(UpstreamAccountGroupMetadata::default());
    };
    let mut conn = pool.acquire().await?;
    Ok(load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default())
}

pub(crate) async fn load_required_account_forward_proxy_scope_from_group_metadata(
    state: &AppState,
    group_name: Option<&str>,
) -> Result<ForwardProxyRouteScope> {
    let normalized_group_name = group_name.map(str::trim).filter(|value| !value.is_empty());
    let Some(group_name) = normalized_group_name else {
        return Ok(ForwardProxyRouteScope::Automatic);
    };
    let bound_proxy_keys = load_group_metadata(&state.pool, Some(group_name))
        .await?
        .bound_proxy_keys;
    let bound_proxy_keys = canonicalize_forward_proxy_bound_keys(state, &bound_proxy_keys).await?;
    required_account_forward_proxy_scope(Some(group_name), bound_proxy_keys)
}

pub(crate) async fn save_group_metadata_record_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
    metadata: UpstreamAccountGroupMetadata,
) -> Result<()> {
    let normalized_note = normalize_optional_text(metadata.note);
    let normalized_bound_proxy_keys = normalize_bound_proxy_keys(metadata.bound_proxy_keys);
    let normalized_node_shunt_enabled = metadata.node_shunt_enabled;
    let normalized_single_account_rotation_enabled = metadata.single_account_rotation_enabled;
    let normalized_upstream_429_retry_enabled = metadata.upstream_429_retry_enabled;
    let normalized_upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
        normalized_upstream_429_retry_enabled,
        metadata.upstream_429_max_retries,
    );
    let normalized_concurrency_limit =
        normalize_concurrency_limit(Some(metadata.concurrency_limit), "concurrencyLimit")
            .map_err(|(status, message)| anyhow!("{status}: {message}"))?;
    let now_iso = format_utc_iso(Utc::now());
    let bound_proxy_keys_json = encode_group_bound_proxy_keys_json(&normalized_bound_proxy_keys)?;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_group_notes (
            group_name,
            note,
            bound_proxy_keys_json,
            node_shunt_enabled,
            single_account_rotation_enabled,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            concurrency_limit,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
        ON CONFLICT(group_name) DO UPDATE SET
            note = excluded.note,
            bound_proxy_keys_json = excluded.bound_proxy_keys_json,
            node_shunt_enabled = excluded.node_shunt_enabled,
            single_account_rotation_enabled = excluded.single_account_rotation_enabled,
            upstream_429_retry_enabled = excluded.upstream_429_retry_enabled,
            upstream_429_max_retries = excluded.upstream_429_max_retries,
            concurrency_limit = excluded.concurrency_limit,
            policy_allow_cut_out = pool_upstream_account_group_notes.policy_allow_cut_out,
            policy_allow_cut_in = pool_upstream_account_group_notes.policy_allow_cut_in,
            policy_priority_tier = pool_upstream_account_group_notes.policy_priority_tier,
            policy_fast_mode_rewrite_mode = pool_upstream_account_group_notes.policy_fast_mode_rewrite_mode,
            policy_image_tool_rewrite_mode = pool_upstream_account_group_notes.policy_image_tool_rewrite_mode,
            policy_codex_imagegen_rewrite_mode = pool_upstream_account_group_notes.policy_codex_imagegen_rewrite_mode,
            policy_concurrency_limit = pool_upstream_account_group_notes.policy_concurrency_limit,
            policy_upstream_429_retry_enabled = pool_upstream_account_group_notes.policy_upstream_429_retry_enabled,
            policy_upstream_429_max_retries = pool_upstream_account_group_notes.policy_upstream_429_max_retries,
            policy_available_models_json = pool_upstream_account_group_notes.policy_available_models_json,
            policy_status_change_upstream_http_401 = pool_upstream_account_group_notes.policy_status_change_upstream_http_401,
            policy_status_change_upstream_http_402 = pool_upstream_account_group_notes.policy_status_change_upstream_http_402,
            policy_status_change_upstream_http_403 = pool_upstream_account_group_notes.policy_status_change_upstream_http_403,
            policy_status_change_reauth_required = pool_upstream_account_group_notes.policy_status_change_reauth_required,
            policy_status_change_upstream_http_429_rate_limit = pool_upstream_account_group_notes.policy_status_change_upstream_http_429_rate_limit,
            policy_status_change_upstream_http_429_quota_exhausted = pool_upstream_account_group_notes.policy_status_change_upstream_http_429_quota_exhausted,
            policy_status_change_usage_snapshot_exhausted = pool_upstream_account_group_notes.policy_status_change_usage_snapshot_exhausted,
            policy_status_change_quota_still_exhausted = pool_upstream_account_group_notes.policy_status_change_quota_still_exhausted,
            policy_status_change_transport_failure = pool_upstream_account_group_notes.policy_status_change_transport_failure,
            policy_status_change_upstream_server_overloaded = pool_upstream_account_group_notes.policy_status_change_upstream_server_overloaded,
            policy_status_change_upstream_http_5xx = pool_upstream_account_group_notes.policy_status_change_upstream_http_5xx,
            policy_responses_first_byte_timeout_secs = pool_upstream_account_group_notes.policy_responses_first_byte_timeout_secs,
            policy_compact_first_byte_timeout_secs = pool_upstream_account_group_notes.policy_compact_first_byte_timeout_secs,
            policy_image_first_byte_timeout_secs = pool_upstream_account_group_notes.policy_image_first_byte_timeout_secs,
            policy_responses_stream_timeout_secs = pool_upstream_account_group_notes.policy_responses_stream_timeout_secs,
            policy_compact_stream_timeout_secs = pool_upstream_account_group_notes.policy_compact_stream_timeout_secs,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(group_name)
    .bind(normalized_note.unwrap_or_default())
    .bind(bound_proxy_keys_json)
    .bind(if normalized_node_shunt_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(if normalized_single_account_rotation_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(if normalized_upstream_429_retry_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(i64::from(normalized_upstream_429_max_retries))
    .bind(normalized_concurrency_limit)
    .bind(now_iso)
    .execute(conn)
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) async fn save_group_note_record(
    pool: &Pool<Sqlite>,
    group_name: &str,
    note: Option<String>,
) -> Result<()> {
    let mut conn = pool.acquire().await?;
    save_group_note_record_conn(&mut conn, group_name, note).await
}

pub(crate) async fn save_group_note_record_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
    note: Option<String>,
) -> Result<()> {
    let mut metadata = load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default();
    metadata.note = note;
    save_group_metadata_record_conn(conn, group_name, metadata).await
}

pub(crate) async fn save_requested_group_metadata_changes(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
    changes: &RequestedGroupMetadataChanges,
) -> Result<()> {
    if !changes.was_requested() {
        return Ok(());
    }
    let Some(group_name) = group_name else {
        return Ok(());
    };
    let mut metadata = load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default();
    if changes.note_was_requested {
        metadata.note = changes.note.clone();
    }
    if changes.bound_proxy_keys_was_requested {
        metadata.bound_proxy_keys = changes.bound_proxy_keys.clone();
    }
    if changes.concurrency_limit_was_requested {
        metadata.concurrency_limit = changes.concurrency_limit;
    }
    if changes.node_shunt_enabled_was_requested {
        metadata.node_shunt_enabled = changes.node_shunt_enabled;
    }
    if changes.single_account_rotation_enabled_was_requested {
        metadata.single_account_rotation_enabled = changes.single_account_rotation_enabled;
    }
    save_group_metadata_record_conn(conn, group_name, metadata).await
}

pub(crate) async fn save_group_metadata_after_account_write(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
    changes: &RequestedGroupMetadataChanges,
    _target_group_already_had_current_account: bool,
) -> Result<()> {
    save_requested_group_metadata_changes(conn, group_name, changes).await
}

pub(crate) async fn cleanup_orphaned_group_metadata(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
) -> Result<()> {
    let _ = conn;
    let _ = group_name;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GroupNodeShuntSlots {
    pub(crate) valid_proxy_keys: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UpstreamAccountNodeShuntAssignments {
    pub(crate) account_proxy_keys: HashMap<i64, String>,
    pub(crate) group_slots: HashMap<String, GroupNodeShuntSlots>,
    pub(crate) group_assigned_proxy_keys: HashMap<String, HashSet<String>>,
    pub(crate) eligible_account_ids: HashSet<i64>,
}

pub(crate) fn compare_node_shunt_reserved_candidates(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
) -> std::cmp::Ordering {
    rhs.in_flight_reservations
        .cmp(&lhs.in_flight_reservations)
        .then_with(|| compare_routing_candidates(lhs, rhs))
}

pub(crate) async fn load_node_shunt_enabled_group_metadata_map(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<String, UpstreamAccountGroupMetadata>> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            group_name,
            note,
            bound_proxy_keys_json,
            node_shunt_enabled,
            single_account_rotation_enabled,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            concurrency_limit
        FROM pool_upstream_account_group_notes
        WHERE node_shunt_enabled != 0
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut groups = HashMap::with_capacity(rows.len());
    for (
        group_name,
        note,
        bound_proxy_keys_json,
        node_shunt_enabled,
        single_account_rotation_enabled,
        upstream_429_retry_enabled,
        upstream_429_max_retries,
        concurrency_limit,
    ) in rows
    {
        let node_shunt_enabled = decode_group_node_shunt_enabled(node_shunt_enabled);
        let single_account_rotation_enabled =
            decode_group_single_account_rotation_enabled(single_account_rotation_enabled);
        let upstream_429_retry_enabled =
            decode_group_upstream_429_retry_enabled(upstream_429_retry_enabled);
        let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
            upstream_429_retry_enabled,
            decode_group_upstream_429_max_retries(upstream_429_max_retries),
        );
        groups.insert(
            group_name,
            UpstreamAccountGroupMetadata {
                note: normalize_optional_text(Some(note)),
                bound_proxy_keys: decode_group_bound_proxy_keys_json(
                    bound_proxy_keys_json.as_deref(),
                ),
                node_shunt_enabled,
                single_account_rotation_enabled,
                upstream_429_retry_enabled,
                upstream_429_max_retries,
                concurrency_limit,
            },
        );
    }
    Ok(groups)
}

pub(crate) async fn load_upstream_account_rows_for_groups(
    pool: &Pool<Sqlite>,
    group_names: &[String],
) -> Result<Vec<UpstreamAccountRow>> {
    if group_names.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS}
        FROM pool_upstream_accounts
        WHERE COALESCE(deleted_at, '') = '' AND group_name IN (
        "#
    ));
    {
        let mut separated = query.separated(", ");
        for group_name in group_names {
            separated.push_bind(group_name);
        }
    }
    query
        .push(") AND kind = ")
        .push_bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .push(" ORDER BY id ASC");

    query
        .build_query_as::<UpstreamAccountRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) fn account_is_node_shunt_slot_eligible(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    if !is_routing_eligible_account(row) {
        return false;
    }
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
    if health_status != UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL {
        return false;
    }
    let sync_state = derive_upstream_account_sync_state(row.enabled != 0, &row.status);
    if sync_state != UPSTREAM_ACCOUNT_SYNC_STATE_IDLE {
        return false;
    }
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
    matches!(
        work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_WORKING
            | UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
            | UPSTREAM_ACCOUNT_WORK_STATUS_IDLE
    )
}

pub(crate) async fn build_upstream_account_node_shunt_assignments(
    state: &AppState,
) -> Result<UpstreamAccountNodeShuntAssignments> {
    let group_metadata_map = load_node_shunt_enabled_group_metadata_map(&state.pool).await?;
    if group_metadata_map.is_empty() {
        return Ok(UpstreamAccountNodeShuntAssignments::default());
    }

    let mut assignments = UpstreamAccountNodeShuntAssignments::default();
    let rows_by_id = load_node_shunt_rows_by_id(&state.pool, &group_metadata_map).await?;
    populate_node_shunt_group_slots(state, &group_metadata_map, &mut assignments).await;

    let reservation_snapshot = pool_routing_reservation_snapshot(state);
    let (group_candidates, candidate_effective_rules) = collect_node_shunt_candidates(
        state,
        &group_metadata_map,
        &rows_by_id,
        Utc::now(),
        &mut assignments,
        &reservation_snapshot,
    )
    .await?;
    let (reserved_candidates, fresh_candidates) =
        sort_node_shunt_candidates(&group_candidates, &candidate_effective_rules);

    let mut globally_occupied_proxy_keys = HashSet::new();
    let mut assigned_account_ids = HashSet::new();
    for (group_name, candidate) in &reserved_candidates {
        let Some(valid_proxy_keys) = assignments
            .group_slots
            .get(group_name)
            .map(|slots| slots.valid_proxy_keys.clone())
        else {
            continue;
        };
        let reserved_proxy_keys = reservation_snapshot.pinned_proxy_keys_for_account(
            candidate.id,
            &valid_proxy_keys,
            &globally_occupied_proxy_keys,
        );
        let Some(proxy_key) = reserved_proxy_keys.first().cloned() else {
            continue;
        };
        if !assigned_account_ids.insert(candidate.id) {
            continue;
        }
        assignments
            .account_proxy_keys
            .insert(candidate.id, proxy_key.clone());
        for reserved_proxy_key in reserved_proxy_keys {
            globally_occupied_proxy_keys.insert(reserved_proxy_key.clone());
            assignments
                .group_assigned_proxy_keys
                .entry(group_name.clone())
                .or_default()
                .insert(reserved_proxy_key);
        }
    }
    for (group_name, slots) in &assignments.group_slots {
        let globally_reserved_proxy_keys =
            reservation_snapshot.reserved_proxy_keys_for_group(&slots.valid_proxy_keys);
        for proxy_key in globally_reserved_proxy_keys {
            globally_occupied_proxy_keys.insert(proxy_key.clone());
            assignments
                .group_assigned_proxy_keys
                .entry(group_name.clone())
                .or_default()
                .insert(proxy_key);
        }
    }
    for (group_name, candidate) in fresh_candidates {
        if !assigned_account_ids.insert(candidate.id) {
            continue;
        }
        let Some(valid_proxy_keys) = assignments
            .group_slots
            .get(&group_name)
            .map(|slots| slots.valid_proxy_keys.clone())
        else {
            continue;
        };
        let Some(proxy_key) = valid_proxy_keys
            .into_iter()
            .find(|proxy_key| !globally_occupied_proxy_keys.contains(proxy_key.as_str()))
        else {
            continue;
        };
        globally_occupied_proxy_keys.insert(proxy_key.clone());
        assignments
            .account_proxy_keys
            .insert(candidate.id, proxy_key.clone());
        assignments
            .group_assigned_proxy_keys
            .entry(group_name)
            .or_default()
            .insert(proxy_key);
    }

    Ok(assignments)
}

async fn load_node_shunt_rows_by_id(
    pool: &Pool<Sqlite>,
    group_metadata_map: &HashMap<String, UpstreamAccountGroupMetadata>,
) -> Result<HashMap<i64, UpstreamAccountRow>> {
    let group_names = group_metadata_map.keys().cloned().collect::<Vec<_>>();
    Ok(load_upstream_account_rows_for_groups(pool, &group_names)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect())
}

async fn populate_node_shunt_group_slots(
    state: &AppState,
    group_metadata_map: &HashMap<String, UpstreamAccountGroupMetadata>,
    assignments: &mut UpstreamAccountNodeShuntAssignments,
) {
    let manager = state.forward_proxy.lock().await;
    for (group_name, metadata) in group_metadata_map {
        assignments.group_slots.insert(
            group_name.clone(),
            GroupNodeShuntSlots {
                valid_proxy_keys: manager
                    .selectable_bound_proxy_keys_in_order(&metadata.bound_proxy_keys),
            },
        );
    }
}
