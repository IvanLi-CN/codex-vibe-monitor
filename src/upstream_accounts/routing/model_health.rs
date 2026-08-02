use super::*;

pub(crate) const MODEL_ROUTE_STATE_AVAILABLE: &str = "available";
pub(crate) const MODEL_ROUTE_STATE_DEGRADED: &str = "degraded";
pub(crate) const MODEL_ROUTE_STATE_COOLING_DOWN: &str = "cooling_down";
pub(crate) const MODEL_ROUTE_PRIORITY_NORMAL: &str = "normal";
pub(crate) const MODEL_ROUTE_PRIORITY_DEMOTED: &str = "demoted";
pub(crate) const MODEL_ROUTE_PRIORITY_EXCLUDED: &str = "excluded";
pub(crate) const MODEL_ROUTE_RETENTION_DAYS: i64 = 7;
pub(crate) const MODEL_ROUTE_FAILURE_THRESHOLD: i64 = 5;
pub(crate) const MODEL_ROUTE_FAILURE_WINDOW_SECS: i64 = 30;
pub(crate) const MODEL_ROUTE_COOLDOWN_BASE_SECS: i64 = 15;
pub(crate) const MODEL_ROUTE_COOLDOWN_MAX_SECS: i64 = 60;

#[derive(Debug, Clone, FromRow)]
struct ModelRouteRow {
    account_id: i64,
    model: String,
    state: String,
    priority: String,
    consecutive_failures: i64,
    streak_started_at: Option<String>,
    changed_at: Option<String>,
    last_seen_at: String,
    last_success_at: Option<String>,
    last_failure_at: Option<String>,
    last_failure_kind: Option<String>,
    last_failure_message: Option<String>,
    cooldown_until: Option<String>,
    reset_fence_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct AttemptRouteContext {
    request_model: Option<String>,
    started_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelRoutePenalty {
    Normal,
    Demoted,
    Excluded,
}

impl ModelRoutePenalty {
    pub(crate) fn score(self) -> i64 {
        match self {
            Self::Normal => 0,
            Self::Demoted => 1,
            Self::Excluded => 2,
        }
    }
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}

fn cutoff_string() -> String {
    (Utc::now() - chrono::Duration::days(MODEL_ROUTE_RETENTION_DAYS)).to_rfc3339()
}

fn account_is_api_key(kind: Option<&str>) -> bool {
    kind == Some(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
}

fn effective_row_state(
    row: &ModelRouteRow,
    now: DateTime<Utc>,
) -> (String, String, Option<String>) {
    if row.state == MODEL_ROUTE_STATE_COOLING_DOWN {
        if let Some(until) = row
            .cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            && until > now
        {
            return (
                MODEL_ROUTE_STATE_COOLING_DOWN.to_string(),
                MODEL_ROUTE_PRIORITY_EXCLUDED.to_string(),
                row.cooldown_until.clone(),
            );
        }
        return (
            MODEL_ROUTE_STATE_DEGRADED.to_string(),
            MODEL_ROUTE_PRIORITY_DEMOTED.to_string(),
            None,
        );
    }
    (
        row.state.clone(),
        row.priority.clone(),
        row.cooldown_until.clone(),
    )
}

fn model_state_from_row(row: ModelRouteRow) -> ModelRoutingState {
    let now = Utc::now();
    let (state, priority, cooldown_until) = effective_row_state(&row, now);
    let cooldown_expired_at = row
        .cooldown_until
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .filter(|until| *until <= now);
    ModelRoutingState {
        model: row.model,
        state,
        priority,
        failure_count: row.consecutive_failures,
        changed_at: if row.state == MODEL_ROUTE_STATE_COOLING_DOWN {
            cooldown_expired_at
                .map(|until| until.to_rfc3339())
                .or(row.changed_at)
        } else {
            row.changed_at
        },
        last_seen_at: row.last_seen_at,
        last_failure_at: row.last_failure_at,
        last_failure_kind: row.last_failure_kind,
        last_failure_message: row.last_failure_message,
        cooldown_until,
    }
}

async fn load_account_kind(pool: &Pool<Sqlite>, account_id: i64) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT kind FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub(crate) async fn purge_model_routes(pool: &Pool<Sqlite>) -> Result<u64> {
    let result =
        sqlx::query("DELETE FROM pool_upstream_account_model_routes WHERE last_seen_at < ?1")
            .bind(cutoff_string())
            .execute(pool)
            .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn count_expired_model_routes(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_model_routes WHERE last_seen_at < ?1",
    )
    .bind(cutoff_string())
    .fetch_one(pool)
    .await?)
}

pub(crate) async fn load_model_routing_states(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Vec<ModelRoutingState>> {
    purge_model_routes(pool).await?;
    let rows = sqlx::query_as::<_, ModelRouteRow>(
        r#"
        SELECT account_id, model, state, priority, consecutive_failures,
               streak_started_at, changed_at, last_seen_at, last_success_at,
               last_failure_at, last_failure_kind, last_failure_message, cooldown_until,
               reset_fence_at
          FROM pool_upstream_account_model_routes
         WHERE account_id = ?1 AND last_seen_at >= ?2
         ORDER BY model COLLATE NOCASE ASC
        "#,
    )
    .bind(account_id)
    .bind(cutoff_string())
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(model_state_from_row).collect())
}

pub(crate) async fn model_route_penalty(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: Option<&str>,
) -> Result<ModelRoutePenalty> {
    let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(ModelRoutePenalty::Normal);
    };
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(ModelRoutePenalty::Normal);
    }
    let row = sqlx::query_as::<_, ModelRouteRow>(
        r#"
        SELECT account_id, model, state, priority, consecutive_failures,
               streak_started_at, changed_at, last_seen_at, last_success_at,
               last_failure_at, last_failure_kind, last_failure_message, cooldown_until,
               reset_fence_at
          FROM pool_upstream_account_model_routes
         WHERE account_id = ?1 AND model = ?2 AND last_seen_at >= ?3
        "#,
    )
    .bind(account_id)
    .bind(model)
    .bind(cutoff_string())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(ModelRoutePenalty::Normal);
    };
    let (state, _, _) = effective_row_state(&row, Utc::now());
    Ok(match state.as_str() {
        MODEL_ROUTE_STATE_COOLING_DOWN => ModelRoutePenalty::Excluded,
        MODEL_ROUTE_STATE_DEGRADED => ModelRoutePenalty::Demoted,
        _ => ModelRoutePenalty::Normal,
    })
}

pub(crate) async fn load_model_route_penalties(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    model: Option<&str>,
) -> Result<HashMap<i64, ModelRoutePenalty>> {
    let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(HashMap::new());
    };
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT routes.account_id, routes.state, routes.priority, routes.cooldown_until \
         FROM pool_upstream_account_model_routes routes \
         INNER JOIN pool_upstream_accounts accounts ON accounts.id = routes.account_id \
         WHERE accounts.kind = ",
    );
    query.push_bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX);
    query
        .push(" AND COALESCE(accounts.deleted_at, '') = '' AND routes.model = ")
        .push_bind(model);
    query
        .push(" AND routes.last_seen_at >= ")
        .push_bind(cutoff_string());
    query.push(" AND routes.account_id IN (");
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(")");
    let rows = query.build().fetch_all(pool).await?;
    let now = Utc::now();
    let mut penalties = HashMap::new();
    for row in rows {
        let account_id = row.try_get::<i64, _>("account_id")?;
        let state = row.try_get::<String, _>("state")?;
        let cooldown_until = row.try_get::<Option<String>, _>("cooldown_until")?;
        let penalty = if state == MODEL_ROUTE_STATE_COOLING_DOWN
            && cooldown_until
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_some_and(|until| until > now)
        {
            ModelRoutePenalty::Excluded
        } else if state == MODEL_ROUTE_STATE_DEGRADED || state == MODEL_ROUTE_STATE_COOLING_DOWN {
            ModelRoutePenalty::Demoted
        } else {
            ModelRoutePenalty::Normal
        };
        penalties.insert(account_id, penalty);
    }
    Ok(penalties)
}

async fn load_attempt_route_context(
    pool: &Pool<Sqlite>,
    attempt_id: i64,
) -> Result<Option<AttemptRouteContext>> {
    let Some(mut context) = sqlx::query_as::<_, AttemptRouteContext>(
        "SELECT request_model, started_at FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    context.request_model = context
        .request_model
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(Some(context))
}

fn rate_limit_subject_is_model_specific(lower: &str, requested_model: Option<&str>) -> bool {
    let Some((_, subject)) = lower.split_once("rate limit reached for ") else {
        return false;
    };
    let subject = subject
        .trim()
        .trim_matches(|character| matches!(character, '"' | '\'' | '`' | '(' | '['));
    let subject = subject.strip_prefix("model ").unwrap_or(subject).trim();
    if subject.is_empty() {
        return false;
    }

    if let Some(requested_model) = requested_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let requested_model = requested_model.to_ascii_lowercase();
        let Some(suffix) = subject.strip_prefix(&requested_model) else {
            return false;
        };
        if suffix.is_empty() || suffix.starts_with(':') {
            return true;
        }
        let generic_scope = [
            "account",
            "organization",
            "org ",
            "project",
            "api key",
            "user",
            "ip ",
            "address",
            "endpoint",
            "route",
            "workspace",
            "subscription",
            "plan ",
            "quota",
            "limit",
            "global",
            "this ",
        ];
        return suffix.starts_with(' ')
            && !generic_scope
                .iter()
                .any(|scope| suffix.trim_start().starts_with(scope));
    }

    // When the attempt has no persisted model yet, accept only a subject that does not
    // look like an account, project, network, endpoint, or plan-wide limit.
    let generic_scope = [
        "account",
        "organization",
        "org ",
        "project",
        "api key",
        "key ",
        "user",
        "ip ",
        "ip:",
        "address",
        "endpoint",
        "route",
        "workspace",
        "subscription",
        "plan ",
        "quota",
        "global",
        "this ",
    ];
    !generic_scope.iter().any(|scope| subject.starts_with(scope))
}

fn unsupported_model_name_matches_request(
    status: StatusCode,
    message: &str,
    requested_model: &str,
) -> bool {
    let requested_model = requested_model.trim();
    if requested_model.is_empty() || status != StatusCode::BAD_REQUEST {
        return false;
    }
    if extract_unsupported_model_from_route_error(status, message)
        .is_some_and(|extracted| extracted.eq_ignore_ascii_case(requested_model))
    {
        return true;
    }
    let lower = message.to_ascii_lowercase();
    let requested_model = requested_model.to_ascii_lowercase();
    let generic_tokens = ["pool", "response_format", "endpoint", "route", "account"];
    for marker in ["unsupported_model:", "unsupported model:"] {
        for rest in lower.split(marker).skip(1) {
            let token = rest
                .trim()
                .trim_matches(|character| matches!(character, '"' | '\'' | '`'))
                .split(|character: char| {
                    character.is_whitespace() || matches!(character, ',' | ';')
                })
                .next()
                .unwrap_or_default();
            if token == requested_model && !generic_tokens.contains(&token) {
                return true;
            }
        }
    }
    false
}

fn not_found_model_name(message: &str) -> Option<String> {
    static MODEL_NOT_FOUND_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?xi)
            \bmodel(?:\s+id)?\s+['\"`]?([a-z0-9][a-z0-9._-]{0,127})['\"`]?
            \s+(?:does\s+not\s+exist|is\s+not\s+found|was\s+not\s+found)\b
            "#,
        )
        .expect("valid model not-found context regex")
    });
    MODEL_NOT_FOUND_CONTEXT_REGEX
        .captures_iter(message)
        .filter_map(|captures| captures.get(1))
        .map(|value| value.as_str().trim().to_string())
        .find(|value| !value.is_empty())
}

pub(crate) fn is_explicit_model_failure(status: StatusCode, message: Option<&str>) -> bool {
    is_explicit_model_failure_for_model(status, message, None)
}

pub(crate) fn is_explicit_model_failure_for_model(
    status: StatusCode,
    message: Option<&str>,
    requested_model: Option<&str>,
) -> bool {
    if !matches!(
        status,
        StatusCode::OK
            | StatusCode::BAD_REQUEST
            | StatusCode::NOT_FOUND
            | StatusCode::TOO_MANY_REQUESTS
    ) {
        return false;
    }
    let Some(message) = message else { return false };
    let lower = message.to_ascii_lowercase();
    let account_scoped = [
        "account",
        "organization",
        "org",
        "project",
        "api key",
        "user",
    ]
    .iter()
    .any(|scope| lower.contains(scope));
    let model_rate_limit = !account_scoped
        && match requested_model {
            Some(_) if lower.contains("rate limit reached for ") => {
                rate_limit_subject_is_model_specific(&lower, requested_model)
            }
            Some(_) => {
                lower.contains("model_rate_limit")
                    || lower.contains("model_quota_exceeded")
                    || (lower.contains("rate limit")
                        && lower.contains("model")
                        && !lower.contains("for model "))
            }
            None => {
                lower.contains("model_rate_limit")
                    || lower.contains("model_quota_exceeded")
                    || (lower.contains("rate limit") && lower.contains("model"))
                    || rate_limit_subject_is_model_specific(&lower, None)
            }
        };
    let unsupported_model_matches_request = match requested_model {
        Some(requested_model) => {
            unsupported_model_name_matches_request(status, message, requested_model)
        }
        None => extract_unsupported_model_from_route_error(status, message).is_some(),
    };
    let model_not_found_matches_request = match requested_model {
        Some(requested_model) => not_found_model_name(message).map_or_else(
            || lower.contains("model_not_found") || lower.contains("model not found"),
            |extracted| extracted.eq_ignore_ascii_case(requested_model.trim()),
        ),
        None => {
            lower.contains("model_not_found")
                || lower.contains("model not found")
                || (lower.contains("model") && lower.contains("does not exist"))
        }
    };
    (!account_scoped
        && (unsupported_model_matches_request
            || lower.contains("model unavailable")
            || lower.contains("model not available")
            || model_not_found_matches_request))
        || model_rate_limit
        || (!account_scoped
            && (lower.contains("model-specific quota")
                || lower.contains("model quota")
                || lower.contains("model limit")))
}

pub(crate) async fn observe_model_route_seen(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: Option<&str>,
) -> Result<()> {
    let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(());
    }
    let now = now_string();
    let cutoff = cutoff_string();
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5) ON CONFLICT(account_id, model) DO UPDATE SET state = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN excluded.state ELSE pool_upstream_account_model_routes.state END, priority = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN excluded.priority ELSE pool_upstream_account_model_routes.priority END, consecutive_failures = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN 0 ELSE pool_upstream_account_model_routes.consecutive_failures END, streak_started_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.streak_started_at END, changed_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN excluded.changed_at ELSE pool_upstream_account_model_routes.changed_at END, last_seen_at = excluded.last_seen_at, last_success_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.last_success_at END, last_failure_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.last_failure_at END, last_failure_kind = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.last_failure_kind END, last_failure_message = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.last_failure_message END, cooldown_until = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.cooldown_until END, reset_fence_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.reset_fence_at END",
    )
    .bind(account_id)
    .bind(model)
    .bind(MODEL_ROUTE_STATE_AVAILABLE)
    .bind(MODEL_ROUTE_PRIORITY_NORMAL)
    .bind(now)
    .bind(cutoff)
    .execute(pool)
    .await?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Structured model route events mirror the persisted event contract."
)]
async fn persist_model_event(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: Option<i64>,
    model: &str,
    action: &str,
    source: &str,
    result: &str,
    state_before: Option<&str>,
    state_after: Option<&str>,
    priority_before: Option<&str>,
    priority_after: Option<&str>,
    failure_count: i64,
    cooldown_until: Option<&str>,
    reason_message: Option<&str>,
    reason_code: Option<&str>,
    http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
) -> Result<()> {
    let now = now_string();
    let sanitized_reason_message = reason_message.and_then(sanitize_account_action_message);
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, result, result_description,
            reason_code, reason_message, http_status, failure_kind, attempt_id, model,
            model_route_state_before, model_route_state_after,
            model_route_priority_before, model_route_priority_after,
            model_route_failure_count, model_route_cooldown_until, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?2)
        "#,
    )
    .bind(account_id)
    .bind(&now)
    .bind(action)
    .bind(source)
    .bind(result)
    .bind(sanitized_reason_message.as_deref())
    .bind(reason_code.unwrap_or("model_route"))
    .bind(sanitized_reason_message.as_deref())
    .bind(http_status.map(|status| i64::from(status.as_u16())))
    .bind(failure_kind)
    .bind(attempt_id)
    .bind(model)
    .bind(state_before)
    .bind(state_after)
    .bind(priority_before)
    .bind(priority_after)
    .bind(failure_count)
    .bind(cooldown_until)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn record_model_route_success_from_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: i64,
    request_started_at: Option<&str>,
) -> Result<()> {
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(());
    }
    let Some(attempt_context) = load_attempt_route_context(pool, attempt_id).await? else {
        return Ok(());
    };
    let Some(model) = attempt_context.request_model else {
        return Ok(());
    };
    let now = now_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    // Publish the successful terminal fence atomically with model recovery so an
    // older concurrent failure cannot slip through before the outer finalizer.
    sqlx::query("UPDATE pool_upstream_request_attempts SET status = 'success' WHERE id = ?1")
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(&model)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        sqlx::query(
            "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at, last_success_at) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5, ?5) ON CONFLICT(account_id, model) DO UPDATE SET last_seen_at = excluded.last_seen_at, last_success_at = excluded.last_success_at",
        )
        .bind(account_id)
        .bind(&model)
        .bind(MODEL_ROUTE_STATE_AVAILABLE)
        .bind(MODEL_ROUTE_PRIORITY_NORMAL)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(());
    };
    if let (Some(started), Some(last_failure)) =
        (request_started_at, row.last_failure_at.as_deref())
        && parse_to_utc_datetime(last_failure).is_some_and(|failure| {
            parse_to_utc_datetime(started).is_some_and(|request| failure > request)
        })
    {
        return Ok(());
    }
    if request_started_at
        .and_then(parse_to_utc_datetime)
        .zip(
            row.reset_fence_at
                .as_deref()
                .and_then(parse_to_utc_datetime),
        )
        .is_some_and(|(request, reset)| request <= reset)
    {
        tx.commit().await?;
        return Ok(());
    }
    let (before_state, before_priority, _) = effective_row_state(&row, Utc::now());
    let changed = before_state != MODEL_ROUTE_STATE_AVAILABLE
        || before_priority != MODEL_ROUTE_PRIORITY_NORMAL
        || row.consecutive_failures != 0
        || row.cooldown_until.is_some();
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, consecutive_failures = 0, streak_started_at = NULL, changed_at = CASE WHEN ?5 = 1 THEN ?2 ELSE changed_at END, last_seen_at = ?2, last_success_at = ?2, last_failure_at = NULL, last_failure_kind = NULL, last_failure_message = NULL, cooldown_until = NULL WHERE account_id = ?1 AND model = ?6",
    )
    .bind(account_id)
    .bind(&now)
    .bind(MODEL_ROUTE_STATE_AVAILABLE)
    .bind(MODEL_ROUTE_PRIORITY_NORMAL)
    .bind(if changed { 1 } else { 0 })
    .bind(&model)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if changed {
        persist_model_event(
            pool,
            account_id,
            Some(attempt_id),
            &model,
            UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            "recovered",
            Some(&before_state),
            Some(MODEL_ROUTE_STATE_AVAILABLE),
            Some(&before_priority),
            Some(MODEL_ROUTE_PRIORITY_NORMAL),
            0,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn record_model_route_failure_from_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: i64,
    status: StatusCode,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
) -> Result<()> {
    record_model_route_failure_from_attempt_with_start(
        pool,
        account_id,
        attempt_id,
        status,
        error_message,
        failure_kind,
        None,
    )
    .await
}

pub(crate) async fn record_temporary_model_route_failure_from_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: i64,
    status: StatusCode,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    reason_code: &str,
) -> Result<bool> {
    record_model_route_failure_from_attempt_inner(
        pool,
        account_id,
        attempt_id,
        status,
        error_message,
        failure_kind,
        None,
        false,
        Some(reason_code),
    )
    .await
}

pub(crate) async fn attempt_has_explicit_model_failure(
    pool: &Pool<Sqlite>,
    attempt_id: i64,
    status: StatusCode,
    error_message: Option<&str>,
) -> Result<bool> {
    let Some(attempt_context) = load_attempt_route_context(pool, attempt_id).await? else {
        return Ok(false);
    };
    let Some(model) = attempt_context.request_model else {
        return Ok(false);
    };
    Ok(is_explicit_model_failure_for_model(
        status,
        error_message,
        Some(&model),
    ))
}

pub(crate) async fn record_model_route_failure_from_attempt_with_start(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: i64,
    status: StatusCode,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    request_started_at: Option<&str>,
) -> Result<()> {
    record_model_route_failure_from_attempt_inner(
        pool,
        account_id,
        attempt_id,
        status,
        error_message,
        failure_kind,
        request_started_at,
        true,
        None,
    )
    .await
    .map(|_| ())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Model failure evidence and concurrency fencing are one state transition."
)]
async fn record_model_route_failure_from_attempt_inner(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: i64,
    status: StatusCode,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    request_started_at: Option<&str>,
    require_explicit_model_failure: bool,
    reason_code: Option<&str>,
) -> Result<bool> {
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(false);
    }
    let Some(attempt_context) = load_attempt_route_context(pool, attempt_id).await? else {
        return Ok(false);
    };
    let Some(model) = attempt_context.request_model else {
        return Ok(false);
    };
    if require_explicit_model_failure
        && !is_explicit_model_failure_for_model(status, error_message, Some(&model))
    {
        return Ok(false);
    }
    let attempt_started_at = request_started_at
        .and_then(parse_to_utc_datetime)
        .or_else(|| {
            attempt_context
                .started_at
                .as_deref()
                .and_then(parse_to_utc_datetime)
        });
    let now = now_string();
    let sanitized_error_message = error_message.and_then(sanitize_account_action_message);
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let existing = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(&model)
    .fetch_optional(&mut *tx)
    .await?;
    let (before_state, before_priority, before_cooldown, previous_failures, streak_started) =
        existing
            .as_ref()
            .map(|row| {
                let (state, priority, cooldown) = effective_row_state(row, Utc::now());
                (
                    state,
                    priority,
                    cooldown,
                    row.consecutive_failures,
                    row.streak_started_at.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    MODEL_ROUTE_STATE_AVAILABLE.to_string(),
                    MODEL_ROUTE_PRIORITY_NORMAL.to_string(),
                    None,
                    0,
                    None,
                )
            });
    if let Some(attempt_started_at) = attempt_started_at {
        let latest_success = existing
            .as_ref()
            .and_then(|row| row.last_success_at.as_deref())
            .and_then(parse_to_utc_datetime);
        if latest_success.is_some_and(|observed_at| attempt_started_at <= observed_at) {
            let latest_successful_attempt = sqlx::query_as::<_, (i64, Option<String>)>(
                "SELECT id, started_at FROM pool_upstream_request_attempts WHERE upstream_account_id = ?1 AND request_model = ?2 COLLATE NOCASE AND status = 'success' ORDER BY id DESC LIMIT 1",
            )
            .bind(account_id)
            .bind(&model)
            .fetch_optional(&mut *tx)
            .await?;
            let newer_success_exists = latest_successful_attempt
                .and_then(|(id, started_at)| {
                    started_at
                        .as_deref()
                        .and_then(parse_to_utc_datetime)
                        .map(|started_at| (id, started_at))
                })
                .is_some_and(|(id, started_at)| {
                    started_at > attempt_started_at
                        || (started_at == attempt_started_at && id > attempt_id)
                });
            if newer_success_exists {
                tx.commit().await?;
                return Ok(false);
            }
        }
    }
    if attempt_started_at
        .zip(
            existing
                .as_ref()
                .and_then(|row| row.reset_fence_at.as_deref())
                .and_then(parse_to_utc_datetime),
        )
        .is_some_and(|(request, reset)| request <= reset)
    {
        tx.commit().await?;
        return Ok(false);
    }
    let within_window = existing
        .as_ref()
        .and_then(|row| row.last_failure_at.as_deref())
        .and_then(parse_to_utc_datetime)
        .is_some_and(|last| (Utc::now() - last).num_seconds() <= MODEL_ROUTE_FAILURE_WINDOW_SECS);
    let failures = previous_failures.saturating_add(1);
    let streak_started = if within_window {
        streak_started.unwrap_or_else(|| now.clone())
    } else {
        now.clone()
    };
    let streak_age = parse_to_utc_datetime(&streak_started)
        .map(|started| (Utc::now() - started).num_seconds())
        .unwrap_or_default();
    let cooling =
        failures >= MODEL_ROUTE_FAILURE_THRESHOLD || streak_age >= MODEL_ROUTE_FAILURE_WINDOW_SECS;
    let cooldown_until = if cooling {
        Some(
            (Utc::now()
                + chrono::Duration::seconds(
                    (MODEL_ROUTE_COOLDOWN_BASE_SECS
                        * (1_i64
                            << failures
                                .saturating_sub(MODEL_ROUTE_FAILURE_THRESHOLD)
                                .min(5)))
                    .min(MODEL_ROUTE_COOLDOWN_MAX_SECS),
                ))
            .to_rfc3339(),
        )
    } else {
        None
    };
    let state = if cooling {
        MODEL_ROUTE_STATE_COOLING_DOWN
    } else {
        MODEL_ROUTE_STATE_DEGRADED
    };
    let priority = if cooling {
        MODEL_ROUTE_PRIORITY_EXCLUDED
    } else {
        MODEL_ROUTE_PRIORITY_DEMOTED
    };
    let changed =
        before_state != state || before_priority != priority || before_cooldown != cooldown_until;
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7, ?8, ?9, ?10) ON CONFLICT(account_id, model) DO UPDATE SET state = excluded.state, priority = excluded.priority, consecutive_failures = excluded.consecutive_failures, streak_started_at = excluded.streak_started_at, changed_at = CASE WHEN ?11 = 1 THEN excluded.changed_at ELSE pool_upstream_account_model_routes.changed_at END, last_seen_at = excluded.last_seen_at, last_failure_at = excluded.last_failure_at, last_failure_kind = excluded.last_failure_kind, last_failure_message = excluded.last_failure_message, cooldown_until = excluded.cooldown_until",
    )
    .bind(account_id)
    .bind(&model)
    .bind(state)
    .bind(priority)
    .bind(failures)
    .bind(&streak_started)
    .bind(&now)
    .bind(failure_kind)
    .bind(sanitized_error_message.as_deref())
    .bind(cooldown_until.as_deref())
    .bind(if changed { 1 } else { 0 })
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if changed {
        persist_model_event(
            pool,
            account_id,
            Some(attempt_id),
            &model,
            if cooling {
                UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_COOLDOWN
            } else {
                UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_DEGRADED
            },
            UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            "failed",
            Some(&before_state),
            Some(state),
            Some(&before_priority),
            Some(priority),
            failures,
            cooldown_until.as_deref(),
            sanitized_error_message.as_deref(),
            reason_code,
            Some(status),
            failure_kind,
        )
        .await?;
    }
    Ok(true)
}

pub(crate) async fn reset_model_route(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: &str,
) -> Result<Option<ModelRoutingState>> {
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(None);
    }
    let model = model.trim();
    if model.is_empty() {
        return Ok(None);
    }
    let Some(row) = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(pool)
    .await? else { return Ok(None) };
    let (before_state, before_priority, _) = effective_row_state(&row, Utc::now());
    let now = now_string();
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, consecutive_failures = 0, streak_started_at = NULL, changed_at = ?2, reset_fence_at = ?2, last_failure_at = NULL, last_failure_kind = NULL, last_failure_message = NULL, cooldown_until = NULL WHERE account_id = ?1 AND model = ?5",
    )
    .bind(account_id)
    .bind(&now)
    .bind(MODEL_ROUTE_STATE_AVAILABLE)
    .bind(MODEL_ROUTE_PRIORITY_NORMAL)
    .bind(model)
    .execute(pool)
    .await?;
    persist_model_event(
        pool,
        account_id,
        None,
        model,
        UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET,
        "manual",
        "reset",
        Some(&before_state),
        Some(MODEL_ROUTE_STATE_AVAILABLE),
        Some(&before_priority),
        Some(MODEL_ROUTE_PRIORITY_NORMAL),
        0,
        None,
        Some("model route manually reset"),
        None,
        None,
        None,
    )
    .await?;
    let updated = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(pool)
    .await?;
    Ok(updated.map(model_state_from_row))
}
