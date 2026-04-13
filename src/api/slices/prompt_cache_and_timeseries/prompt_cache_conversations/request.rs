use super::*;

pub(crate) async fn fetch_prompt_cache_conversations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PromptCacheConversationsQuery>,
) -> Result<Json<PromptCacheConversationsResponse>, ApiError> {
    let request = resolve_prompt_cache_conversations_request(params)?;
    let response =
        if request.page_size.is_none() && request.cursor.is_none() && request.snapshot_at.is_none()
        {
            let response =
                fetch_prompt_cache_conversations_cached(state.as_ref(), request.selection).await?;
            match request.detail_level {
                PromptCacheConversationDetailLevel::Full => response,
                PromptCacheConversationDetailLevel::Compact => {
                    compact_prompt_cache_conversations_response(response)
                }
            }
        } else {
            build_prompt_cache_conversations_response_for_request(state.as_ref(), request).await?
        };
    Ok(Json(response))
}

pub(crate) fn normalize_prompt_cache_conversation_limit(raw: Option<i64>) -> i64 {
    match raw {
        Some(value @ (20 | 50 | 100)) => value,
        _ => PROMPT_CACHE_CONVERSATION_DEFAULT_LIMIT,
    }
}

pub(crate) fn normalize_prompt_cache_conversation_activity_hours(raw: Option<i64>) -> Option<i64> {
    match raw {
        Some(value @ (1 | 3 | 6 | 12 | 24)) => Some(value),
        _ => None,
    }
}

pub(crate) fn normalize_prompt_cache_conversation_activity_minutes(
    raw: Option<i64>,
) -> Option<i64> {
    match raw {
        Some(5) => Some(5),
        _ => None,
    }
}

pub(crate) fn resolve_prompt_cache_conversation_selection(
    params: PromptCacheConversationsQuery,
) -> Result<PromptCacheConversationSelection, ApiError> {
    let activity_param_count =
        i64::from(params.activity_hours.is_some()) + i64::from(params.activity_minutes.is_some());
    if params.limit.is_some() && activity_param_count > 0 {
        return Err(ApiError::bad_request(anyhow!(
            "limit, activityHours, and activityMinutes are mutually exclusive"
        )));
    }
    if params.activity_hours.is_some() && params.activity_minutes.is_some() {
        return Err(ApiError::bad_request(anyhow!(
            "activityHours and activityMinutes are mutually exclusive"
        )));
    }

    if let Some(hours) = normalize_prompt_cache_conversation_activity_hours(params.activity_hours) {
        return Ok(PromptCacheConversationSelection::ActivityWindowHours(hours));
    }

    if let Some(minutes) =
        normalize_prompt_cache_conversation_activity_minutes(params.activity_minutes)
    {
        return Ok(PromptCacheConversationSelection::ActivityWindowMinutes(
            minutes,
        ));
    }

    Ok(PromptCacheConversationSelection::Count(
        normalize_prompt_cache_conversation_limit(params.limit),
    ))
}

fn normalize_prompt_cache_conversation_page_size(
    raw: Option<i64>,
) -> Result<Option<i64>, ApiError> {
    let Some(value) = raw else {
        return Ok(None);
    };
    if !(1..=100).contains(&value) {
        return Err(ApiError::bad_request(anyhow!(
            "pageSize must be between 1 and 100"
        )));
    }
    Ok(Some(value))
}

fn resolve_prompt_cache_conversation_detail_level(
    raw: Option<&str>,
) -> Result<PromptCacheConversationDetailLevel, ApiError> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(PromptCacheConversationDetailLevel::Full);
    };
    match value {
        "full" => Ok(PromptCacheConversationDetailLevel::Full),
        "compact" => Ok(PromptCacheConversationDetailLevel::Compact),
        _ => Err(ApiError::bad_request(anyhow!(
            "detail must be one of: full, compact"
        ))),
    }
}

fn resolve_prompt_cache_conversations_request(
    params: PromptCacheConversationsQuery,
) -> Result<PromptCacheConversationsRequest, ApiError> {
    let selection = resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit: params.limit,
        activity_hours: params.activity_hours,
        activity_minutes: params.activity_minutes,
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
    })?;
    let detail_level = resolve_prompt_cache_conversation_detail_level(params.detail.as_deref())?;
    let normalized_page_size = normalize_prompt_cache_conversation_page_size(params.page_size)?;
    let uses_pagination =
        normalized_page_size.is_some() || params.cursor.is_some() || params.snapshot_at.is_some();

    if params.cursor.is_some() && params.snapshot_at.is_none() {
        return Err(ApiError::bad_request(anyhow!("cursor requires snapshotAt")));
    }

    if uses_pagination
        && !matches!(
            selection,
            PromptCacheConversationSelection::ActivityWindowMinutes(_)
        )
    {
        return Err(ApiError::bad_request(anyhow!(
            "pageSize, cursor, and snapshotAt are only supported for activityMinutes working conversations"
        )));
    }

    Ok(PromptCacheConversationsRequest {
        selection,
        detail_level,
        page_size: if uses_pagination {
            Some(normalized_page_size.unwrap_or(20))
        } else {
            normalized_page_size
        },
        cursor: params.cursor,
        snapshot_at: params.snapshot_at,
    })
}

pub(crate) fn resolve_prompt_cache_conversation_snapshot_at_with_default(
    raw: Option<&str>,
    default_now: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(default_now);
    };
    parse_to_utc_datetime(value).ok_or_else(|| anyhow!("invalid snapshotAt: {value}"))
}

pub(crate) fn resolve_prompt_cache_conversation_snapshot_at(
    raw: Option<&str>,
) -> Result<DateTime<Utc>> {
    resolve_prompt_cache_conversation_snapshot_at_with_default(raw, Utc::now())
}

fn resolve_working_conversation_sort_anchor<'a>(
    last_terminal_at: Option<&'a str>,
    last_in_flight_at: Option<&'a str>,
    created_at: &'a str,
) -> &'a str {
    match (last_terminal_at, last_in_flight_at) {
        (Some(last_terminal_at), Some(last_in_flight_at)) => {
            if last_terminal_at >= last_in_flight_at {
                last_terminal_at
            } else {
                last_in_flight_at
            }
        }
        (Some(last_terminal_at), None) => last_terminal_at,
        (None, Some(last_in_flight_at)) => last_in_flight_at,
        (None, None) => created_at,
    }
}

fn encode_prompt_cache_conversation_cursor(
    sort_anchor_at: &str,
    created_at: &str,
    prompt_cache_key: &str,
    snapshot_boundary_row_id_ceiling: Option<i64>,
) -> String {
    let payload = serde_json::to_vec(&(
        sort_anchor_at,
        created_at,
        prompt_cache_key,
        snapshot_boundary_row_id_ceiling,
    ))
    .expect("prompt cache cursor payload should serialize");
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload)
}

pub(crate) fn decode_prompt_cache_conversation_cursor(
    raw: &str,
) -> Result<(String, String, String, Option<i64>)> {
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        raw.trim(),
    )
    .map_err(|err| anyhow!("invalid cursor encoding: {err}"))?;
    if let Ok((sort_anchor_at, created_at, prompt_cache_key, snapshot_boundary_row_id_ceiling)) =
        serde_json::from_slice::<(String, String, String, Option<i64>)>(&decoded)
    {
        let sort_anchor_at = sort_anchor_at.trim();
        let created_at = created_at.trim();
        let prompt_cache_key = prompt_cache_key.trim();
        if sort_anchor_at.is_empty() || created_at.is_empty() || prompt_cache_key.is_empty() {
            return Err(anyhow!("invalid cursor payload"));
        }
        return Ok((
            sort_anchor_at.to_string(),
            created_at.to_string(),
            prompt_cache_key.to_string(),
            snapshot_boundary_row_id_ceiling,
        ));
    }

    let decoded =
        String::from_utf8(decoded).map_err(|err| anyhow!("invalid cursor bytes: {err}"))?;
    let mut parts = decoded.splitn(4, '|');
    let Some(sort_anchor_at) = parts.next() else {
        return Err(anyhow!("invalid cursor payload"));
    };
    let Some(created_at) = parts.next() else {
        return Err(anyhow!("invalid cursor payload"));
    };
    let Some(prompt_cache_key) = parts.next() else {
        return Err(anyhow!("invalid cursor payload"));
    };
    let sort_anchor_at = sort_anchor_at.trim();
    let created_at = created_at.trim();
    let prompt_cache_key = prompt_cache_key.trim();
    if sort_anchor_at.is_empty() || created_at.is_empty() || prompt_cache_key.is_empty() {
        return Err(anyhow!("invalid cursor payload"));
    }
    let snapshot_boundary_row_id_ceiling = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|err| anyhow!("invalid cursor snapshot row id ceiling: {err}"))
        })
        .transpose()?;
    Ok((
        sort_anchor_at.to_string(),
        created_at.to_string(),
        prompt_cache_key.to_string(),
        snapshot_boundary_row_id_ceiling,
    ))
}

pub(crate) fn build_prompt_cache_conversation_cursor(
    row: &PromptCacheConversationAggregateRow,
    snapshot_boundary_row_id_ceiling: Option<i64>,
) -> String {
    let sort_anchor_at = row.sort_anchor_at.as_deref().unwrap_or_else(|| {
        resolve_working_conversation_sort_anchor(
            row.last_terminal_at.as_deref(),
            row.last_in_flight_at.as_deref(),
            row.created_at.as_str(),
        )
    });
    encode_prompt_cache_conversation_cursor(
        sort_anchor_at,
        &row.created_at,
        &row.prompt_cache_key,
        snapshot_boundary_row_id_ceiling,
    )
}

async fn query_prompt_cache_conversation_snapshot_row_id_ceiling(
    pool: &Pool<Sqlite>,
    snapshot_at: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<Option<i64>> {
    let snapshot_created_at_upper_bound = format_utc_iso_precise(snapshot_at);
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT MAX(id) AS max_id \
         FROM codex_invocations \
         WHERE julianday(created_at) <= julianday(",
    );
    query.push_bind(snapshot_created_at_upper_bound).push(")");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    let (max_id,) = query
        .build_query_as::<(Option<i64>,)>()
        .fetch_one(pool)
        .await?;
    Ok(max_id)
}

pub(crate) async fn resolve_prompt_cache_conversation_snapshot_filter(
    pool: &Pool<Sqlite>,
    snapshot_at: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    cursor_snapshot_boundary_row_id_ceiling: Option<i64>,
) -> Result<PromptCacheConversationSnapshotFilter> {
    let snapshot_upper_bound = db_occurred_at_lower_bound(snapshot_at + ChronoDuration::seconds(1));
    let snapshot_boundary_row_id_ceiling = Some(match cursor_snapshot_boundary_row_id_ceiling {
        Some(value) => value,
        None => {
            query_prompt_cache_conversation_snapshot_row_id_ceiling(pool, snapshot_at, source_scope)
                .await?
                .unwrap_or(0)
        }
    });
    Ok(PromptCacheConversationSnapshotFilter {
        snapshot_upper_bound,
        snapshot_boundary_row_id_ceiling,
    })
}
