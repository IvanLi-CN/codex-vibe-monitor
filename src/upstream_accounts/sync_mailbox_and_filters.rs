include!("sync_mailbox_and_filters/part_01.rs");
include!("sync_mailbox_and_filters/part_02.rs");
pub(crate) enum KaisouMailAttachReadState<T> {
    Readable(T),
    NotReadable,
}

pub(crate) fn kaisoumail_attach_status_is_not_readable(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    )
}

pub(crate) async fn resolve_mailbox_message_state_for_attach(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    messages: &[KaisouMailMessageSummary],
) -> Result<KaisouMailAttachReadState<(Option<ParsedMailboxCode>, Option<ParsedMailboxInvite>)>> {
    let mut latest_code = None;
    let mut latest_invite = None;
    for summary in messages.iter() {
        if latest_code.is_some() && latest_invite.is_some() {
            break;
        }
        let detail = match kaisoumail_get_message_for_attach(client, config, &summary.id).await? {
            KaisouMailAttachReadState::Readable(detail) => detail,
            KaisouMailAttachReadState::NotReadable => {
                return Ok(KaisouMailAttachReadState::NotReadable);
            }
        };
        if latest_code.is_none() {
            latest_code = parse_mailbox_code(&detail);
        }
        if latest_invite.is_none() {
            latest_invite = parse_mailbox_invite(&detail);
        }
    }

    Ok(KaisouMailAttachReadState::Readable((
        latest_code,
        latest_invite,
    )))
}

pub(crate) async fn kaisoumail_create_mailbox(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
) -> Result<KaisouMailMailboxPayload> {
    let response = client
        .post(
            config
                .base_url
                .join("/api/mailboxes")
                .context("invalid kaisoumail mailbox create endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .json(&json!({
            "expiresInMinutes": DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS / 60,
        }))
        .send()
        .await
        .context("failed to create kaisoumail mailbox")?
        .error_for_status()
        .context("kaisoumail mailbox creation request failed")?;

    response
        .json::<KaisouMailMailboxPayload>()
        .await
        .context("failed to decode kaisoumail create mailbox response")
}

pub(crate) async fn kaisoumail_ensure_mailbox_for_address(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    email_address: &str,
) -> Result<KaisouMailMailboxPayload> {
    let requested_email = normalize_mailbox_address(email_address)
        .ok_or_else(|| anyhow!("manual kaisoumail address must not be blank"))?;
    let response = client
        .post(
            config
                .base_url
                .join("/api/mailboxes/ensure")
                .context("invalid kaisoumail mailbox ensure endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .json(&json!({
            "address": requested_email,
            "expiresInMinutes": DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS / 60,
        }))
        .send()
        .await
        .context("failed to ensure kaisoumail mailbox")?
        .error_for_status()
        .context("kaisoumail mailbox ensure request failed")?;

    let payload = response
        .json::<KaisouMailMailboxPayload>()
        .await
        .context("failed to decode kaisoumail ensure mailbox response")?;
    validate_kaisoumail_mailbox_address_matches_request(&payload, &requested_email)?;
    Ok(payload)
}

pub(crate) async fn kaisoumail_get_meta(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
) -> Result<KaisouMailMetaPayload> {
    let response = client
        .get(
            config
                .base_url
                .join("/api/meta")
                .context("invalid kaisoumail meta endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .context("failed to load kaisoumail config")?
        .error_for_status()
        .context("kaisoumail config request failed")?;

    response
        .json::<KaisouMailMetaPayload>()
        .await
        .context("failed to decode kaisoumail meta response")
}

pub(crate) async fn kaisoumail_list_mailboxes(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
) -> Result<Vec<KaisouMailMailboxSummary>> {
    let response = client
        .get(
            config
                .base_url
                .join("/api/mailboxes")
                .context("invalid kaisoumail mailbox list endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .context("failed to list kaisoumail mailboxes")?
        .error_for_status()
        .context("kaisoumail mailbox list request failed")?;
    let payload = response
        .json::<KaisouMailMailboxListPayload>()
        .await
        .context("failed to decode kaisoumail mailbox list response")?;
    Ok(payload.mailboxes)
}

pub(crate) async fn kaisoumail_list_messages(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    mailbox_address: &str,
) -> Result<Vec<KaisouMailMessageSummary>> {
    let mut url = config
        .base_url
        .join("/api/messages")
        .context("invalid kaisoumail message list endpoint")?;
    url.query_pairs_mut()
        .append_pair("mailbox", mailbox_address);
    let response = client
        .get(url)
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to list kaisoumail messages for {mailbox_address}"))?
        .error_for_status()
        .with_context(|| {
            format!("kaisoumail list messages request failed for {mailbox_address}")
        })?;

    let payload = response
        .json::<KaisouMailMessageListPayload>()
        .await
        .context("failed to decode kaisoumail message list response")?;
    Ok(payload.messages)
}

pub(crate) async fn kaisoumail_list_messages_for_attach(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    mailbox_address: &str,
) -> Result<KaisouMailAttachReadState<Vec<KaisouMailMessageSummary>>> {
    let mut url = config
        .base_url
        .join("/api/messages")
        .context("invalid kaisoumail message list endpoint")?;
    url.query_pairs_mut()
        .append_pair("mailbox", mailbox_address);
    let response = client
        .get(url)
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to list kaisoumail messages for {mailbox_address}"))?;
    if kaisoumail_attach_status_is_not_readable(response.status()) {
        return Ok(KaisouMailAttachReadState::NotReadable);
    }
    let response = response.error_for_status().with_context(|| {
        format!("kaisoumail list messages request failed for {mailbox_address}")
    })?;

    let payload = response
        .json::<KaisouMailMessageListPayload>()
        .await
        .context("failed to decode kaisoumail message list response")?;
    Ok(KaisouMailAttachReadState::Readable(payload.messages))
}

pub(crate) async fn kaisoumail_get_message(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    message_id: &str,
) -> Result<KaisouMailMessageDetail> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/messages/{message_id}"))
                .context("invalid kaisoumail message detail endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to load kaisoumail message {message_id}"))?
        .error_for_status()
        .with_context(|| format!("kaisoumail message request failed for {message_id}"))?;

    let payload = response
        .json::<KaisouMailMessageDetailPayload>()
        .await
        .context("failed to decode kaisoumail message detail response")?;
    Ok(payload.message)
}

pub(crate) async fn kaisoumail_get_message_for_attach(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    message_id: &str,
) -> Result<KaisouMailAttachReadState<KaisouMailMessageDetail>> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/messages/{message_id}"))
                .context("invalid kaisoumail message detail endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to load kaisoumail message {message_id}"))?;
    if kaisoumail_attach_status_is_not_readable(response.status()) {
        return Ok(KaisouMailAttachReadState::NotReadable);
    }
    let response = response
        .error_for_status()
        .with_context(|| format!("kaisoumail message request failed for {message_id}"))?;

    let payload = response
        .json::<KaisouMailMessageDetailPayload>()
        .await
        .context("failed to decode kaisoumail message detail response")?;
    Ok(KaisouMailAttachReadState::Readable(payload.message))
}

pub(crate) async fn kaisoumail_delete_mailbox(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    remote_email_id: &str,
) -> Result<()> {
    client
        .delete(
            config
                .base_url
                .join(&format!("/api/mailboxes/{remote_email_id}"))
                .context("invalid kaisoumail delete endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to delete kaisoumail mailbox {remote_email_id}"))?
        .error_for_status()
        .with_context(|| format!("kaisoumail delete request failed for {remote_email_id}"))?;
    Ok(())
}

pub(crate) async fn refresh_oauth_mailbox_session_status(
    state: &AppState,
    row: &OauthMailboxSessionRow,
) -> Result<OauthMailboxSessionRow> {
    let config = upstream_mailbox_config(&state.config).map_err(|(_, message)| anyhow!(message))?;
    let mut messages =
        kaisoumail_list_messages(&state.http_clients.shared, config, &row.email_address).await?;
    sort_mailbox_messages_desc(&mut messages);

    let unseen_messages = collect_unseen_mailbox_messages(messages, row.last_message_id.as_deref());
    let (fresh_code, fresh_invite) =
        resolve_mailbox_message_state(&state.http_clients.shared, config, &unseen_messages).await?;
    let latest_code = merge_mailbox_code(fresh_code, parsed_code_from_mailbox_row(row));
    let latest_invite = merge_mailbox_invite(fresh_invite, parsed_invite_from_mailbox_row(row));
    let next_last_message_id =
        next_mailbox_cursor_after_refresh(row.last_message_id.as_deref(), &unseen_messages);

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_mailbox_sessions
        SET latest_code_value = ?2,
            latest_code_source = ?3,
            latest_code_updated_at = ?4,
            invite_subject = ?5,
            invite_copy_value = ?6,
            invite_copy_label = ?7,
            invite_updated_at = ?8,
            invited = ?9,
            last_message_id = ?10,
            updated_at = ?11
        WHERE session_id = ?1
        "#,
    )
    .bind(&row.session_id)
    .bind(latest_code.as_ref().map(|value| value.value.clone()))
    .bind(latest_code.as_ref().map(|value| value.source.clone()))
    .bind(latest_code.as_ref().map(|value| value.updated_at.clone()))
    .bind(latest_invite.as_ref().map(|value| value.subject.clone()))
    .bind(latest_invite.as_ref().map(|value| value.copy_value.clone()))
    .bind(latest_invite.as_ref().map(|value| value.copy_label.clone()))
    .bind(latest_invite.as_ref().map(|value| value.updated_at.clone()))
    .bind(if latest_invite.is_some() { 1 } else { 0 })
    .bind(next_last_message_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await?;

    load_oauth_mailbox_session(&state.pool, &row.session_id)
        .await?
        .ok_or_else(|| anyhow!("mailbox session disappeared after status refresh"))
}

pub(crate) fn normalize_tag_name(value: &str) -> Result<String, (StatusCode, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "tag name is required".to_string()));
    }
    if trimmed.chars().count() > 48 {
        return Err((
            StatusCode::BAD_REQUEST,
            "tag name must be 48 characters or fewer".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn normalize_bulk_upstream_account_ids(
    account_ids: &[i64],
) -> Result<Vec<i64>, (StatusCode, String)> {
    let mut normalized = account_ids
        .iter()
        .copied()
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "accountIds must contain at least one positive integer".to_string(),
        ));
    }
    Ok(normalized)
}

pub(crate) fn normalize_upstream_account_list_page(value: Option<usize>) -> usize {
    value.filter(|page| *page > 0).unwrap_or(1)
}

pub(crate) fn normalize_upstream_account_list_page_size(value: Option<usize>) -> usize {
    value
        .filter(|page_size| UPSTREAM_ACCOUNT_LIST_PAGE_SIZE_OPTIONS.contains(page_size))
        .unwrap_or(DEFAULT_UPSTREAM_ACCOUNT_LIST_PAGE_SIZE)
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LegacyUpstreamAccountStatusFilter {
    pub(crate) work_status: Option<&'static str>,
    pub(crate) enable_status: Option<&'static str>,
    pub(crate) health_status: Option<&'static str>,
    pub(crate) sync_state: Option<&'static str>,
}

pub(crate) fn normalize_upstream_account_work_status_filter(
    value: Option<&str>,
) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_WORK_STATUS_WORKING => Some(UPSTREAM_ACCOUNT_WORK_STATUS_WORKING),
        UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED => Some(UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED),
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE => Some(UPSTREAM_ACCOUNT_WORK_STATUS_IDLE),
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED => {
            Some(UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED)
        }
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE => Some(UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE),
        _ => None,
    }
}

pub(crate) fn normalize_upstream_account_enable_status_filter(
    value: Option<&str>,
) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED => Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
        UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED => Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
        _ => None,
    }
}

pub(crate) fn normalize_upstream_account_health_status_filter(
    value: Option<&str>,
) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL => Some(UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL),
        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH => Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE)
        }
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED)
        }
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER | UPSTREAM_ACCOUNT_STATUS_ERROR => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER)
        }
        _ => None,
    }
}

pub(crate) fn collect_normalized_upstream_account_filters(
    values: &[String],
    legacy_value: Option<&'static str>,
    normalize: fn(Option<&str>) -> Option<&'static str>,
) -> Vec<&'static str> {
    let mut normalized = Vec::new();

    for value in values {
        let Some(next_value) = normalize(Some(value.as_str())) else {
            continue;
        };
        if !normalized.contains(&next_value) {
            normalized.push(next_value);
        }
    }

    if normalized.is_empty()
        && let Some(legacy_value) = legacy_value
    {
        normalized.push(legacy_value);
    }

    normalized
}

pub(crate) fn normalize_legacy_upstream_account_status_filter(
    value: Option<&str>,
) -> LegacyUpstreamAccountStatusFilter {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    match normalized.as_deref() {
        Some(UPSTREAM_ACCOUNT_STATUS_ACTIVE) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            health_status: Some(UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_STATUS_SYNCING) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            health_status: Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER) | Some(UPSTREAM_ACCOUNT_STATUS_ERROR) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_STATUS_DISABLED) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        _ => LegacyUpstreamAccountStatusFilter::default(),
    }
}

pub(crate) fn normalize_bulk_upstream_account_action(
    value: &str,
) -> Result<String, (StatusCode, String)> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE
        | BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE
        | BULK_UPSTREAM_ACCOUNT_ACTION_DELETE
        | BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP => Ok(normalized),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "unsupported bulk action".to_string(),
        )),
    }
}

pub(crate) fn normalize_tag_rule(
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: Option<&str>,
    fast_mode_rewrite_mode: Option<&str>,
    concurrency_limit: Option<i64>,
    upstream_429_retry_enabled: Option<bool>,
    upstream_429_max_retries: Option<u8>,
    available_models: Option<Vec<String>>,
) -> Result<TagRoutingRule, (StatusCode, String)> {
    let priority_tier = normalize_tag_priority_tier(priority_tier)?;
    let fast_mode_rewrite_mode = normalize_tag_fast_mode_rewrite_mode(fast_mode_rewrite_mode)?;
    let concurrency_limit = normalize_concurrency_limit(concurrency_limit, "concurrencyLimit")?;
    let upstream_429_retry_enabled = upstream_429_retry_enabled.unwrap_or(false);
    let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
        upstream_429_retry_enabled,
        upstream_429_max_retries
            .map(normalize_group_upstream_429_max_retries)
            .unwrap_or_default(),
    );
    Ok(TagRoutingRule {
        allow_cut_out,
        allow_cut_in,
        priority_tier,
        fast_mode_rewrite_mode,
        concurrency_limit,
        upstream_429_retry_enabled,
        upstream_429_max_retries,
        available_models: normalize_available_models(available_models, "availableModels")?,
    })
}

pub(crate) fn normalize_group_account_routing_rule(
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: Option<&str>,
    fast_mode_rewrite_mode: Option<&str>,
    image_tool_rewrite_mode: Option<&str>,
    concurrency_limit: Option<i64>,
    upstream_429_retry_enabled: Option<bool>,
    upstream_429_max_retries: Option<u8>,
    available_models: Option<Vec<String>>,
) -> Result<GroupAccountRoutingRule, (StatusCode, String)> {
    let available_models_defined = available_models.is_some();
    let priority_tier = normalize_tag_priority_tier(priority_tier)?;
    let fast_mode_rewrite_mode = normalize_tag_fast_mode_rewrite_mode(fast_mode_rewrite_mode)?;
    let image_tool_rewrite_mode = normalize_image_tool_rewrite_mode(image_tool_rewrite_mode)?;
    let concurrency_limit = normalize_concurrency_limit(concurrency_limit, "concurrencyLimit")?;
    let upstream_429_retry_enabled = upstream_429_retry_enabled.unwrap_or(false);
    let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
        upstream_429_retry_enabled,
        upstream_429_max_retries
            .map(normalize_group_upstream_429_max_retries)
            .unwrap_or_default(),
    );
    Ok(GroupAccountRoutingRule {
        allow_cut_out,
        allow_cut_in,
        priority_tier,
        fast_mode_rewrite_mode,
        image_tool_rewrite_mode,
        codex_imagegen_rewrite_mode: None,
        request_compression_algorithm: None,
        concurrency_limit,
        upstream_429_retry_enabled,
        upstream_429_max_retries,
        available_models: normalize_available_models(available_models, "availableModels")?,
        available_models_mode: available_models_defined.then_some(AvailableModelsMode::Allowlist),
        available_models_defined,
        status_change_reasons: default_status_change_reasons(),
        timeouts: None,
    })
}

pub(crate) fn normalize_available_models(
    value: Option<Vec<String>>,
    field_name: &str,
) -> Result<Vec<String>, (StatusCode, String)> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for model in value.unwrap_or_default() {
        let model = model.trim();
        if model.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("{field_name} entries must be non-empty"),
            ));
        }
        if seen.insert(model.to_string()) {
            normalized.push(model.to_string());
        }
    }
    Ok(normalized)
}

pub(crate) fn normalize_tag_priority_tier(
    value: Option<&str>,
) -> Result<TagPriorityTier, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("normal");
    match normalized {
        "fallback" => Ok(TagPriorityTier::Fallback),
        "normal" => Ok(TagPriorityTier::Normal),
        "primary" => Ok(TagPriorityTier::Primary),
        "no_new" => Ok(TagPriorityTier::NoNew),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "priorityTier must be one of: primary, normal, fallback, no_new".to_string(),
        )),
    }
}

pub(crate) fn decode_tag_priority_tier(value: &str) -> TagPriorityTier {
    match value.trim() {
        "no_new" => TagPriorityTier::NoNew,
        "fallback" => TagPriorityTier::Fallback,
        "primary" => TagPriorityTier::Primary,
        _ => TagPriorityTier::Normal,
    }
}

pub(crate) fn normalize_tag_fast_mode_rewrite_mode(
    value: Option<&str>,
) -> Result<TagFastModeRewriteMode, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("keep_original");
    match normalized {
        "force_remove" => Ok(TagFastModeRewriteMode::ForceRemove),
        "keep_original" => Ok(TagFastModeRewriteMode::KeepOriginal),
        "fill_missing" => Ok(TagFastModeRewriteMode::FillMissing),
        "force_add" => Ok(TagFastModeRewriteMode::ForceAdd),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "fastModeRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add"
                .to_string(),
        )),
    }
}

pub(crate) fn decode_tag_fast_mode_rewrite_mode(value: &str) -> TagFastModeRewriteMode {
    match value.trim() {
        "force_remove" => TagFastModeRewriteMode::ForceRemove,
        "fill_missing" => TagFastModeRewriteMode::FillMissing,
        "force_add" => TagFastModeRewriteMode::ForceAdd,
        _ => TagFastModeRewriteMode::KeepOriginal,
    }
}

pub(crate) fn normalize_image_tool_rewrite_mode(
    value: Option<&str>,
) -> Result<ImageToolRewriteMode, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("keep_original");
    match normalized {
        "force_remove" => Ok(ImageToolRewriteMode::ForceRemove),
        "keep_original" => Ok(ImageToolRewriteMode::KeepOriginal),
        "fill_missing" => Ok(ImageToolRewriteMode::FillMissing),
        "force_add" => Ok(ImageToolRewriteMode::ForceAdd),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "imageToolRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add".to_string(),
        )),
    }
}

pub(crate) fn decode_image_tool_rewrite_mode(value: &str) -> ImageToolRewriteMode {
    ImageToolRewriteMode::from_str(value)
}

pub(crate) fn normalize_codex_imagegen_rewrite_mode(
    value: Option<&str>,
) -> Result<CodexImagegenRewriteMode, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("keep_original");
    match normalized {
        "force_remove" => Ok(CodexImagegenRewriteMode::ForceRemove),
        "keep_original" => Ok(CodexImagegenRewriteMode::KeepOriginal),
        "fill_missing" => Ok(CodexImagegenRewriteMode::FillMissing),
        "force_add" => Ok(CodexImagegenRewriteMode::ForceAdd),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "codexImagegenRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add".to_string(),
        )),
    }
}

pub(crate) fn decode_codex_imagegen_rewrite_mode(value: &str) -> CodexImagegenRewriteMode {
    CodexImagegenRewriteMode::from_str(value)
}

pub(crate) fn normalize_request_compression_algorithm(
    value: Option<&str>,
) -> Result<RequestCompressionAlgorithm, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("identity");
    match normalized {
        "follow" => Ok(RequestCompressionAlgorithm::Follow),
        "identity" => Ok(RequestCompressionAlgorithm::Identity),
        "gzip" => Ok(RequestCompressionAlgorithm::Gzip),
        "deflate" => Ok(RequestCompressionAlgorithm::Deflate),
        "zstd" => Ok(RequestCompressionAlgorithm::Zstd),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "requestCompressionAlgorithm must be one of: follow, identity, gzip, deflate, zstd"
                .to_string(),
        )),
    }
}

pub(crate) fn decode_request_compression_algorithm(value: &str) -> RequestCompressionAlgorithm {
    RequestCompressionAlgorithm::from_str(value)
}

pub(crate) fn normalize_request_compression_level_preset(
    value: Option<&str>,
) -> Result<RequestCompressionLevelPreset, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("balanced");
    match normalized {
        "fast" => Ok(RequestCompressionLevelPreset::Fast),
        "balanced" => Ok(RequestCompressionLevelPreset::Balanced),
        "best" => Ok(RequestCompressionLevelPreset::Best),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "requestCompressionLevelPreset must be one of: fast, balanced, best".to_string(),
        )),
    }
}

pub(crate) fn decode_request_compression_level_preset(
    value: Option<&str>,
) -> RequestCompressionLevelPreset {
    value
        .map(RequestCompressionLevelPreset::from_str)
        .unwrap_or_default()
}

pub(crate) fn decode_capability_support(value: Option<&str>) -> CapabilitySupport {
    value
        .map(CapabilitySupport::from_str)
        .unwrap_or(CapabilitySupport::Unknown)
}

pub(crate) fn decode_capability_override(value: Option<&str>) -> Option<CapabilitySupport> {
    let capability = decode_capability_support(value);
    (!matches!(capability, CapabilitySupport::Unknown)).then_some(capability)
}

pub(crate) fn effective_capability_support(
    observed: CapabilitySupport,
    override_value: Option<CapabilitySupport>,
) -> CapabilitySupport {
    override_value.unwrap_or(observed)
}

pub(crate) fn normalize_concurrency_limit(
    value: Option<i64>,
    field_name: &str,
) -> Result<i64, (StatusCode, String)> {
    let value = value.unwrap_or(0);
    if !(0..=30).contains(&value) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be between 0 and 30"),
        ));
    }
    Ok(value)
}

pub(crate) fn parse_tag_ids_json(raw: Option<&str>) -> Vec<i64> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<i64>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|value| *value > 0)
        .collect()
}

pub(crate) fn encode_tag_ids_json(tag_ids: &[i64]) -> Result<String> {
    serde_json::to_string(tag_ids).context("failed to encode tag ids")
}

pub(crate) fn parse_string_array_json(raw: Option<&str>) -> Vec<String> {
    parse_string_array_json_with_invalid(raw).0
}

pub(crate) fn parse_string_array_json_with_invalid(raw: Option<&str>) -> (Vec<String>, bool) {
    let Some(raw) = raw else {
        return (Vec::new(), false);
    };
    let parsed = match serde_json::from_str::<Vec<String>>(raw) {
        Ok(parsed) => parsed,
        Err(_) => return (Vec::new(), true),
    };
    let invalid_entry = parsed
        .iter()
        .any(|value| normalize_optional_text(Some(value.clone())).is_none());
    let normalized = parsed
        .into_iter()
        .filter_map(|value| normalize_optional_text(Some(value)))
        .collect();
    (normalized, invalid_entry)
}
pub(crate) fn encode_string_array_json(values: &[String]) -> Result<String> {
    serde_json::to_string(values).context("failed to encode string array")
}

pub(crate) fn account_tag_summary_from_row(row: &AccountTagRow) -> AccountTagSummary {
    let (available_models, available_models_invalid) =
        parse_string_array_json_with_invalid(row.available_models_json.as_deref());
    AccountTagSummary {
        id: row.tag_id,
        name: row.name.clone(),
        routing_rule: TagRoutingRule {
            allow_cut_out: row.allow_cut_out != 0,
            allow_cut_in: row.allow_cut_in != 0,
            priority_tier: decode_tag_priority_tier(&row.priority_tier),
            fast_mode_rewrite_mode: decode_tag_fast_mode_rewrite_mode(&row.fast_mode_rewrite_mode),
            concurrency_limit: row.concurrency_limit,
            upstream_429_retry_enabled: decode_group_upstream_429_retry_enabled(
                row.upstream_429_retry_enabled,
            ),
            upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(
                decode_group_upstream_429_retry_enabled(row.upstream_429_retry_enabled),
                decode_group_upstream_429_max_retries(row.upstream_429_max_retries),
            ),
            available_models,
        },
        available_models_invalid,
        system_key: row.system_key.clone(),
        protected: row.protected != 0,
    }
}

pub(crate) fn tag_summary_from_row(row: &TagListRow) -> TagSummary {
    TagSummary {
        id: row.id,
        name: row.name.clone(),
        routing_rule: TagRoutingRule {
            allow_cut_out: row.allow_cut_out != 0,
            allow_cut_in: row.allow_cut_in != 0,
            priority_tier: decode_tag_priority_tier(&row.priority_tier),
            fast_mode_rewrite_mode: decode_tag_fast_mode_rewrite_mode(&row.fast_mode_rewrite_mode),
            concurrency_limit: row.concurrency_limit,
            upstream_429_retry_enabled: decode_group_upstream_429_retry_enabled(
                row.upstream_429_retry_enabled,
            ),
            upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(
                decode_group_upstream_429_retry_enabled(row.upstream_429_retry_enabled),
                decode_group_upstream_429_max_retries(row.upstream_429_max_retries),
            ),
            available_models: parse_string_array_json(row.available_models_json.as_deref()),
        },
        account_count: row.account_count,
        group_count: row.group_count,
        updated_at: row.updated_at.clone(),
        system_key: row.system_key.clone(),
        protected: row.protected != 0,
    }
}

pub(crate) fn status_change_reasons_from_columns(
    policy_status_change_upstream_http_401: Option<i64>,
    policy_status_change_upstream_http_402: Option<i64>,
    policy_status_change_upstream_http_403: Option<i64>,
    policy_status_change_reauth_required: Option<i64>,
    policy_status_change_upstream_http_429_rate_limit: Option<i64>,
    policy_status_change_upstream_http_429_quota_exhausted: Option<i64>,
    policy_status_change_usage_snapshot_exhausted: Option<i64>,
    policy_status_change_quota_still_exhausted: Option<i64>,
    policy_status_change_transport_failure: Option<i64>,
    policy_status_change_upstream_server_overloaded: Option<i64>,
    policy_status_change_upstream_http_5xx: Option<i64>,
) -> StatusChangeReasonSettings {
    let mut reasons = default_status_change_reasons();
    for (reason_code, value) in [
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401,
            policy_status_change_upstream_http_401,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402,
            policy_status_change_upstream_http_402,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403,
            policy_status_change_upstream_http_403,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
            policy_status_change_reauth_required,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT,
            policy_status_change_upstream_http_429_rate_limit,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            policy_status_change_upstream_http_429_quota_exhausted,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
            policy_status_change_usage_snapshot_exhausted,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
            policy_status_change_quota_still_exhausted,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
            policy_status_change_transport_failure,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
            policy_status_change_upstream_server_overloaded,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX,
            policy_status_change_upstream_http_5xx,
        ),
    ] {
        if let Some(value) = value {
            reasons.insert(reason_code.to_string(), value != 0);
        }
    }
    reasons
}
