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
const CACHE_HIT_FAILURE_KIND: &str = "cache_hit_rate";
const CACHE_HIT_FAILURE_MESSAGE: &str = "cache hit rate below configured threshold";
const CACHE_USAGE_MISSING_REASON_CODE: &str = "cache_usage_missing";
const CACHE_USAGE_MISSING_MESSAGE: &str =
    "cache usage is unavailable for a cache-protected model route";

struct CacheHitRouteEvent {
    action: &'static str,
    result: &'static str,
    state_before: String,
    state_after: &'static str,
    priority_before: String,
    priority_after: &'static str,
    failure_count: i64,
    cooldown_until: Option<String>,
    reason_message: Option<&'static str>,
    reason_code: Option<&'static str>,
    failure_kind: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ModelRouteCacheObservationOutcome {
    pub(crate) observed: bool,
    pub(crate) availability_increased: bool,
}

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
    #[sqlx(default)]
    cache_concurrency_limit: Option<i64>,
    #[sqlx(default)]
    cache_recovery_limit: Option<i64>,
    #[sqlx(default)]
    cache_low_hit_streak: i64,
    #[sqlx(default)]
    cache_cooldown_level: i64,
    #[sqlx(default)]
    cache_last_hit_rate_percent: Option<i64>,
    #[sqlx(default)]
    cache_usage_missing_since: Option<String>,
    #[sqlx(default)]
    cache_usage_missing_reason: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct ModelRoutingLiveRouteRow {
    account_id: i64,
    account_display_name: String,
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
    #[sqlx(default)]
    cache_concurrency_limit: Option<i64>,
    #[sqlx(default)]
    cache_recovery_limit: Option<i64>,
    #[sqlx(default)]
    cache_low_hit_streak: i64,
    #[sqlx(default)]
    cache_cooldown_level: i64,
    #[sqlx(default)]
    cache_last_hit_rate_percent: Option<i64>,
    #[sqlx(default)]
    cache_usage_missing_since: Option<String>,
    #[sqlx(default)]
    cache_usage_missing_reason: Option<String>,
}

impl ModelRoutingLiveRouteRow {
    fn into_response(self) -> ModelRoutingLiveAccount {
        let route = model_state_from_row(ModelRouteRow {
            account_id: self.account_id,
            model: self.model,
            state: self.state,
            priority: self.priority,
            consecutive_failures: self.consecutive_failures,
            streak_started_at: self.streak_started_at,
            changed_at: self.changed_at,
            last_seen_at: self.last_seen_at,
            last_success_at: self.last_success_at,
            last_failure_at: self.last_failure_at,
            last_failure_kind: self.last_failure_kind,
            last_failure_message: self.last_failure_message,
            cooldown_until: self.cooldown_until,
            reset_fence_at: self.reset_fence_at,
            cache_concurrency_limit: self.cache_concurrency_limit,
            cache_recovery_limit: self.cache_recovery_limit,
            cache_low_hit_streak: self.cache_low_hit_streak,
            cache_cooldown_level: self.cache_cooldown_level,
            cache_last_hit_rate_percent: self.cache_last_hit_rate_percent,
            cache_usage_missing_since: self.cache_usage_missing_since,
            cache_usage_missing_reason: self.cache_usage_missing_reason,
        });
        ModelRoutingLiveAccount {
            account_id: self.account_id,
            account_display_name: (!self.account_display_name.trim().is_empty())
                .then_some(self.account_display_name),
            route,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct AttemptRouteContext {
    request_model: Option<String>,
    upstream_request_model: Option<String>,
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

/// The subset of model-route state that participates in request-time pool
/// selection. It is built with the immutable routing snapshot, then evaluated
/// against the current clock without another database read.
#[derive(Debug, Clone)]
pub(crate) struct ModelRouteRuntimeSnapshot {
    state: String,
    cooldown_until: Option<String>,
    cache_concurrency_limit: Option<i64>,
}

impl ModelRouteRuntimeSnapshot {
    pub(crate) fn penalty_at(&self, now: DateTime<Utc>) -> ModelRoutePenalty {
        let cooling_down = self.state == MODEL_ROUTE_STATE_COOLING_DOWN
            && self
                .cooldown_until
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_some_and(|until| until > now);
        if cooling_down {
            ModelRoutePenalty::Excluded
        } else if self.state == MODEL_ROUTE_STATE_DEGRADED
            || self.state == MODEL_ROUTE_STATE_COOLING_DOWN
        {
            ModelRoutePenalty::Demoted
        } else {
            ModelRoutePenalty::Normal
        }
    }

    pub(crate) fn requires_expired_cooldown_probe_at(&self, now: DateTime<Utc>) -> bool {
        self.state == MODEL_ROUTE_STATE_COOLING_DOWN
            && self
                .cooldown_until
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_some_and(|until| until <= now)
    }

    pub(crate) fn concurrency_limit_at(
        &self,
        cache_hit_protection_enabled: bool,
        now: DateTime<Utc>,
    ) -> Option<i64> {
        if self.requires_expired_cooldown_probe_at(now) {
            return Some(1);
        }
        cache_hit_protection_enabled
            .then_some(self.cache_concurrency_limit)
            .flatten()
            .map(|limit| limit.max(1))
    }
}

pub(crate) async fn load_model_route_runtime_snapshots(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<(i64, String), ModelRouteRuntimeSnapshot>> {
    let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<i64>)>(
        r#"
        SELECT routes.account_id, routes.model, routes.state, routes.cooldown_until,
               routes.cache_concurrency_limit
          FROM pool_upstream_account_model_routes AS routes
          JOIN pool_upstream_accounts AS accounts ON accounts.id = routes.account_id
         WHERE accounts.kind = ?1
           AND COALESCE(accounts.deleted_at, '') = ''
           AND routes.last_seen_at >= ?2
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
    .bind(cutoff_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(account_id, model, state, cooldown_until, cache_concurrency_limit)| {
                (
                    (account_id, model),
                    ModelRouteRuntimeSnapshot {
                        state,
                        cooldown_until,
                        cache_concurrency_limit,
                    },
                )
            },
        )
        .collect())
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

fn is_cache_owned_route(row: &ModelRouteRow) -> bool {
    row.last_failure_kind.as_deref() == Some(CACHE_HIT_FAILURE_KIND)
        || row.cache_concurrency_limit.is_some()
        || row.cache_recovery_limit.is_some()
}

fn cache_protection_controls_route_state(row: &ModelRouteRow) -> bool {
    row.last_failure_kind.as_deref() == Some(CACHE_HIT_FAILURE_KIND)
        || (row.last_failure_kind.is_none()
            && (row.cache_concurrency_limit.is_some() || row.cache_recovery_limit.is_some()))
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
        cache_concurrency_limit: row.cache_concurrency_limit,
        cache_recovery_limit: row.cache_recovery_limit,
        cache_low_hit_streak: row.cache_low_hit_streak,
        cache_cooldown_level: row.cache_cooldown_level,
        cache_last_hit_rate_percent: row.cache_last_hit_rate_percent,
        cache_usage_missing_since: row.cache_usage_missing_since,
        cache_usage_missing_reason: row.cache_usage_missing_reason,
        probe_required: row.state == MODEL_ROUTE_STATE_COOLING_DOWN
            && row
                .cooldown_until
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_some_and(|until| until <= now),
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

pub(crate) async fn purge_model_routes_bounded(pool: &Pool<Sqlite>, limit: usize) -> Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_model_routes
        WHERE rowid IN (
            SELECT rowid
            FROM pool_upstream_account_model_routes
            WHERE last_seen_at < ?1
            ORDER BY last_seen_at ASC, rowid ASC
            LIMIT ?2
        )
        "#,
    )
    .bind(cutoff_string())
    .bind(limit.max(1) as i64)
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
    // Expired rows are filtered by the query below. Physical cleanup belongs to
    // retention, where it is bounded and admitted through the maintenance writer.
    let rows = sqlx::query_as::<_, ModelRouteRow>(
        r#"
        SELECT account_id, model, state, priority, consecutive_failures,
               streak_started_at, changed_at, last_seen_at, last_success_at,
               last_failure_at, last_failure_kind, last_failure_message, cooldown_until,
               reset_fence_at, cache_concurrency_limit, cache_recovery_limit,
               cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent,
               cache_usage_missing_since, cache_usage_missing_reason
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

pub(crate) async fn load_api_key_model_routing_live_accounts(
    pool: &Pool<Sqlite>,
    model_filter: Option<&str>,
    state_filter: Option<&str>,
) -> Result<Vec<ModelRoutingLiveAccount>> {
    let rows = sqlx::query_as::<_, ModelRoutingLiveRouteRow>(
        r#"
        SELECT routes.account_id,
               accounts.display_name AS account_display_name,
               routes.model,
               routes.state,
               routes.priority,
               routes.consecutive_failures,
               routes.streak_started_at,
               routes.changed_at,
               routes.last_seen_at,
               routes.last_success_at,
               routes.last_failure_at,
               routes.last_failure_kind,
               routes.last_failure_message,
               routes.cooldown_until,
               routes.reset_fence_at,
               routes.cache_concurrency_limit,
               routes.cache_recovery_limit,
               routes.cache_low_hit_streak,
               routes.cache_cooldown_level,
               routes.cache_last_hit_rate_percent,
               routes.cache_usage_missing_since,
               routes.cache_usage_missing_reason
          FROM pool_upstream_account_model_routes AS routes
          JOIN pool_upstream_accounts AS accounts ON accounts.id = routes.account_id
         WHERE accounts.kind = ?1
           AND COALESCE(accounts.deleted_at, '') = ''
           AND routes.last_seen_at >= ?2
           AND (?3 IS NULL OR routes.model = ?3)
         ORDER BY routes.model COLLATE NOCASE ASC, routes.account_id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
    .bind(cutoff_string())
    .bind(model_filter)
    .fetch_all(pool)
    .await?;

    let mut accounts = rows
        .into_iter()
        .map(ModelRoutingLiveRouteRow::into_response)
        .collect::<Vec<_>>();
    if let Some(state_filter) = state_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        accounts.retain(|account| account.route.state == state_filter);
    }
    Ok(accounts)
}

pub(crate) async fn model_route_penalty(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: Option<&str>,
) -> Result<ModelRoutePenalty> {
    let Some(model) = model.map(str::trim) else {
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
               reset_fence_at, cache_concurrency_limit, cache_recovery_limit,
               cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent,
               cache_usage_missing_since, cache_usage_missing_reason
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

pub(crate) async fn model_route_concurrency_limit(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: Option<&str>,
) -> Result<Option<i64>> {
    let Some(model) = model.map(str::trim) else {
        return Ok(None);
    };
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(None);
    }
    let row = sqlx::query_as::<_, (String, Option<String>, Option<i64>)>(
        "SELECT state, cooldown_until, cache_concurrency_limit FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(pool)
    .await?;
    let Some((state, cooldown_until, cache_concurrency_limit)) = row else {
        return Ok(None);
    };
    if state == MODEL_ROUTE_STATE_COOLING_DOWN
        && cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|until| until <= Utc::now())
    {
        // Every cooldown recovery is an exclusive single probe, even when
        // cache protection is disabled.
        return Ok(Some(1));
    }
    let settings = resolve_cache_hit_protection_settings(&load_pool_routing_settings(pool).await?);
    Ok(settings
        .enabled
        .then_some(cache_concurrency_limit)
        .flatten()
        .map(|limit| limit.max(1)))
}

pub(crate) async fn model_route_requires_expired_cooldown_probe(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: Option<&str>,
) -> Result<bool> {
    let Some(model) = model.map(str::trim) else {
        return Ok(false);
    };
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(false);
    }
    let row = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT state, cooldown_until FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some_and(|(state, cooldown_until)| {
        state == MODEL_ROUTE_STATE_COOLING_DOWN
            && cooldown_until
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_some_and(|until| until <= Utc::now())
    }))
}

pub(crate) async fn earliest_model_route_cooldown_expiry(
    pool: &Pool<Sqlite>,
    model: Option<&str>,
    account_ids: &[i64],
) -> Result<Option<String>> {
    let Some(model) = model.map(str::trim) else {
        return Ok(None);
    };
    if account_ids.is_empty() {
        return Ok(None);
    }
    let now = Utc::now();
    let cooldowns = sqlx::query_as::<_, (i64, String)>(
        "SELECT account_id, cooldown_until FROM pool_upstream_account_model_routes WHERE model = ?1 AND state = ?2 AND cooldown_until IS NOT NULL",
    )
    .bind(model)
    .bind(MODEL_ROUTE_STATE_COOLING_DOWN)
    .fetch_all(pool)
    .await?;
    Ok(cooldowns
        .into_iter()
        .filter(|(account_id, _)| account_ids.contains(account_id))
        .filter_map(|(_, cooldown)| {
            parse_to_utc_datetime(&cooldown)
                .filter(|until| *until > now)
                .map(|until| (until, cooldown))
        })
        .min_by_key(|(until, _)| *until)
        .map(|(_, cooldown)| cooldown))
}

pub(crate) async fn clear_cache_hit_protection_state(
    pool: &Pool<Sqlite>,
    reason_code: &str,
) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let rows = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE last_failure_kind = ?1 OR cache_concurrency_limit IS NOT NULL OR cache_recovery_limit IS NOT NULL OR cache_low_hit_streak != 0 OR cache_cooldown_level != 0 OR cache_last_hit_rate_percent IS NOT NULL OR cache_usage_missing_since IS NOT NULL",
    )
    .bind(CACHE_HIT_FAILURE_KIND)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        tx.commit().await?;
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE pool_upstream_account_model_routes
           SET state = CASE
                   WHEN last_failure_kind = ?1
                     OR (last_failure_kind IS NULL AND (cache_concurrency_limit IS NOT NULL OR cache_recovery_limit IS NOT NULL))
                   THEN ?2
                   ELSE state
               END,
               priority = CASE
                   WHEN last_failure_kind = ?1
                     OR (last_failure_kind IS NULL AND (cache_concurrency_limit IS NOT NULL OR cache_recovery_limit IS NOT NULL))
                   THEN ?3
                   ELSE priority
               END,
               cooldown_until = CASE
                   WHEN last_failure_kind = ?1
                     OR (last_failure_kind IS NULL AND (cache_concurrency_limit IS NOT NULL OR cache_recovery_limit IS NOT NULL))
                   THEN NULL
                   ELSE cooldown_until
               END,
               last_failure_at = CASE WHEN last_failure_kind = ?1 THEN NULL ELSE last_failure_at END,
               last_failure_kind = CASE WHEN last_failure_kind = ?1 THEN NULL ELSE last_failure_kind END,
               last_failure_message = CASE WHEN last_failure_kind = ?1 THEN NULL ELSE last_failure_message END,
               cache_concurrency_limit = NULL,
               cache_recovery_limit = NULL,
               cache_low_hit_streak = 0,
               cache_cooldown_level = 0,
               cache_last_hit_rate_percent = NULL,
               cache_usage_missing_since = NULL,
               cache_usage_missing_reason = NULL
        "#,
    )
    .bind(CACHE_HIT_FAILURE_KIND)
    .bind(MODEL_ROUTE_STATE_AVAILABLE)
    .bind(MODEL_ROUTE_PRIORITY_NORMAL)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    for row in rows {
        let cache_controlled_state = cache_protection_controls_route_state(&row);
        persist_model_event(
            pool,
            row.account_id,
            None,
            &row.model,
            UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            "cache_hit_settings_cleanup",
            Some(&row.state),
            Some(if cache_controlled_state {
                MODEL_ROUTE_STATE_AVAILABLE
            } else {
                row.state.as_str()
            }),
            Some(&row.priority),
            Some(if cache_controlled_state {
                MODEL_ROUTE_PRIORITY_NORMAL
            } else {
                row.priority.as_str()
            }),
            row.consecutive_failures,
            if cache_controlled_state {
                None
            } else {
                row.cooldown_until.as_deref()
            },
            Some("cache hit protection settings changed"),
            Some(reason_code),
            None,
            None,
        )
        .await?;
    }
    Ok(())
}

async fn mark_model_route_cache_usage_missing(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: &str,
    reason: &'static str,
) -> Result<ModelRouteCacheObservationOutcome> {
    let now = now_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let enabled = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT cache_hit_protection_enabled FROM pool_routing_settings WHERE id = 1",
    )
    .fetch_optional(&mut *tx)
    .await?
    .flatten()
    .unwrap_or_default()
        != 0;
    if !enabled {
        tx.commit().await?;
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let row = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    if !is_cache_owned_route(&row) {
        tx.commit().await?;
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let first_missing_sample = row.cache_usage_missing_since.is_none();
    let (state_before, priority_before, _) = effective_row_state(&row, Utc::now());
    let cooling_is_active = row.state == MODEL_ROUTE_STATE_COOLING_DOWN
        && row
            .cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|until| until > Utc::now());
    let state_after = if cooling_is_active {
        MODEL_ROUTE_STATE_COOLING_DOWN
    } else {
        MODEL_ROUTE_STATE_DEGRADED
    };
    let priority_after = if cooling_is_active {
        MODEL_ROUTE_PRIORITY_EXCLUDED
    } else {
        MODEL_ROUTE_PRIORITY_DEMOTED
    };
    let changed = state_before != state_after
        || priority_before != priority_after
        || row.cache_concurrency_limit != Some(1);
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, changed_at = CASE WHEN ?5 = 1 THEN ?6 ELSE changed_at END, last_seen_at = ?6, cooldown_until = CASE WHEN ?7 = 1 THEN cooldown_until ELSE NULL END, cache_concurrency_limit = 1, cache_recovery_limit = COALESCE(cache_recovery_limit, cache_concurrency_limit, 1), cache_usage_missing_since = COALESCE(cache_usage_missing_since, ?6), cache_usage_missing_reason = COALESCE(cache_usage_missing_reason, ?8) WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .bind(state_after)
    .bind(priority_after)
    .bind(if changed { 1 } else { 0 })
    .bind(&now)
    .bind(if cooling_is_active { 1 } else { 0 })
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    if first_missing_sample {
        warn!(
            account_id,
            model,
            reason,
            cache_usage_missing_since = %now,
            "cache-protected model route remains constrained because cache usage is unavailable"
        );
        persist_model_event(
            pool,
            account_id,
            None,
            model,
            UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            "awaiting_cache_usage",
            Some(&state_before),
            Some(state_after),
            Some(&priority_before),
            Some(priority_after),
            row.consecutive_failures,
            row.cooldown_until.as_deref(),
            Some(CACHE_USAGE_MISSING_MESSAGE),
            Some(CACHE_USAGE_MISSING_REASON_CODE),
            None,
            row.last_failure_kind.as_deref(),
        )
        .await?;
    }
    Ok(ModelRouteCacheObservationOutcome {
        observed: true,
        availability_increased: false,
    })
}

pub(crate) async fn observe_model_route_cache_hit(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: Option<&str>,
    input_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    active_concurrency: i64,
) -> Result<ModelRouteCacheObservationOutcome> {
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let Some(model) = model.map(str::trim) else {
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    let Some(input_tokens) = input_tokens else {
        return mark_model_route_cache_usage_missing(
            pool,
            account_id,
            model,
            "missing_input_tokens",
        )
        .await;
    };
    if input_tokens < CACHE_HIT_PROTECTION_MIN_INPUT_TOKENS as i64 {
        return mark_model_route_cache_usage_missing(
            pool,
            account_id,
            model,
            "input_below_cache_observation_threshold",
        )
        .await;
    }
    let Some(cache_input_tokens) = cache_input_tokens else {
        return mark_model_route_cache_usage_missing(
            pool,
            account_id,
            model,
            "missing_cache_input_tokens",
        )
        .await;
    };
    let cached = cache_input_tokens.clamp(0, input_tokens);
    let hit_rate_percent = cached.saturating_mul(100) / input_tokens;
    let now = now_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let settings = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT cache_hit_protection_enabled, cache_hit_low_rate_threshold_percent FROM pool_routing_settings WHERE id = 1",
    )
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or((Some(0), Some(10)));
    if settings.0.unwrap_or_default() == 0 {
        tx.commit().await?;
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let threshold_percent = settings
        .1
        .and_then(|value| u8::try_from(value).ok())
        .filter(|value| (1..=100).contains(value))
        .unwrap_or(10);
    let low_hit =
        cached.saturating_mul(100) < input_tokens.saturating_mul(i64::from(threshold_percent));
    let row = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    let expired_probe = row.state == MODEL_ROUTE_STATE_COOLING_DOWN
        && row
            .cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|until| until <= Utc::now());
    let active_cooling = row.state == MODEL_ROUTE_STATE_COOLING_DOWN
        && row
            .cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|until| until > Utc::now());
    if active_cooling {
        // Requests already in flight when the combination entered cooldown do
        // not shorten the cooldown or consume the eventual single probe.
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET last_seen_at = ?3, cache_last_hit_rate_percent = ?4, cache_usage_missing_since = NULL, cache_usage_missing_reason = NULL WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(model)
        .bind(&now)
        .bind(hit_rate_percent)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(ModelRouteCacheObservationOutcome {
            observed: true,
            availability_increased: false,
        });
    }
    let (before_state, before_priority, _) = effective_row_state(&row, Utc::now());
    let mut event = None;
    if low_hit {
        let recovery_limit = row
            .cache_recovery_limit
            .unwrap_or_else(|| active_concurrency.max(1));
        let current_limit = row.cache_concurrency_limit.unwrap_or(recovery_limit).max(1);
        let next_limit = (current_limit / 2).max(1);
        let low_hit_streak = if next_limit == 1 {
            row.cache_low_hit_streak.saturating_add(1)
        } else {
            0
        };
        let cooling = low_hit_streak >= 3;
        let cooldown_level = if cooling {
            row.cache_cooldown_level.saturating_add(1).clamp(1, 3)
        } else {
            row.cache_cooldown_level
        };
        let cooldown_until = cooling.then(|| {
            (Utc::now() + chrono::Duration::seconds(15 * (1_i64 << (cooldown_level - 1))))
                .to_rfc3339()
        });
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, changed_at = ?5, last_seen_at = ?5, last_failure_at = ?5, last_failure_kind = ?6, last_failure_message = ?7, cooldown_until = ?8, cache_concurrency_limit = ?9, cache_recovery_limit = ?10, cache_low_hit_streak = ?11, cache_cooldown_level = ?12, cache_last_hit_rate_percent = ?13, cache_usage_missing_since = NULL, cache_usage_missing_reason = NULL WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(model)
        .bind(if cooling { MODEL_ROUTE_STATE_COOLING_DOWN } else { MODEL_ROUTE_STATE_DEGRADED })
        .bind(if cooling { MODEL_ROUTE_PRIORITY_EXCLUDED } else { MODEL_ROUTE_PRIORITY_DEMOTED })
        .bind(&now)
        .bind(CACHE_HIT_FAILURE_KIND)
        .bind(CACHE_HIT_FAILURE_MESSAGE)
        .bind(cooldown_until.as_deref())
        .bind(next_limit)
        .bind(recovery_limit)
        .bind(if cooling { 0 } else { low_hit_streak })
        .bind(cooldown_level)
        .bind(hit_rate_percent)
        .execute(&mut *tx)
        .await?;
        info!(
            account_id,
            model,
            hit_rate_percent,
            next_limit,
            low_hit_streak,
            cooling,
            "applied cache-hit route protection"
        );
        let state_after = if cooling {
            MODEL_ROUTE_STATE_COOLING_DOWN
        } else {
            MODEL_ROUTE_STATE_DEGRADED
        };
        let priority_after = if cooling {
            MODEL_ROUTE_PRIORITY_EXCLUDED
        } else {
            MODEL_ROUTE_PRIORITY_DEMOTED
        };
        event = Some(CacheHitRouteEvent {
            action: if cooling {
                UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_COOLDOWN
            } else {
                UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_DEGRADED
            },
            result: if expired_probe {
                "cache_hit_probe_limited"
            } else {
                "cache_hit_protection"
            },
            state_before: before_state,
            state_after,
            priority_before: before_priority,
            priority_after,
            failure_count: row.consecutive_failures,
            cooldown_until,
            reason_message: Some(CACHE_HIT_FAILURE_MESSAGE),
            reason_code: Some(CACHE_HIT_FAILURE_KIND),
            failure_kind: Some(CACHE_HIT_FAILURE_KIND),
        });
    } else if expired_probe {
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, changed_at = ?5, last_seen_at = ?5, last_failure_at = NULL, last_failure_kind = NULL, last_failure_message = NULL, cooldown_until = NULL, cache_concurrency_limit = NULL, cache_recovery_limit = NULL, cache_low_hit_streak = 0, cache_cooldown_level = 0, cache_last_hit_rate_percent = ?6, cache_usage_missing_since = NULL, cache_usage_missing_reason = NULL WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(model)
        .bind(MODEL_ROUTE_STATE_AVAILABLE)
        .bind(MODEL_ROUTE_PRIORITY_NORMAL)
        .bind(&now)
        .bind(hit_rate_percent)
        .execute(&mut *tx)
        .await?;
        info!(
            account_id,
            model, hit_rate_percent, "cache-hit single probe recovered model route"
        );
        event = Some(CacheHitRouteEvent {
            action: UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED,
            result: "cache_hit_probe_recovered",
            state_before: before_state,
            state_after: MODEL_ROUTE_STATE_AVAILABLE,
            priority_before: before_priority,
            priority_after: MODEL_ROUTE_PRIORITY_NORMAL,
            failure_count: 0,
            cooldown_until: None,
            reason_message: None,
            reason_code: Some(CACHE_HIT_FAILURE_KIND),
            failure_kind: None,
        });
    } else if let Some(current_limit) = row.cache_concurrency_limit {
        let recovery_limit = row.cache_recovery_limit.unwrap_or(current_limit).max(1);
        let next_limit = current_limit.saturating_add(1);
        let recovered = next_limit >= recovery_limit;
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, changed_at = ?5, last_seen_at = ?5, last_failure_at = CASE WHEN ?6 = 1 AND last_failure_kind = ?7 THEN NULL ELSE last_failure_at END, last_failure_kind = CASE WHEN ?6 = 1 AND last_failure_kind = ?7 THEN NULL ELSE last_failure_kind END, last_failure_message = CASE WHEN ?6 = 1 AND last_failure_kind = ?7 THEN NULL ELSE last_failure_message END, cooldown_until = NULL, cache_concurrency_limit = ?8, cache_recovery_limit = ?9, cache_low_hit_streak = 0, cache_cooldown_level = 0, cache_last_hit_rate_percent = ?10, cache_usage_missing_since = NULL, cache_usage_missing_reason = NULL WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(model)
        .bind(if recovered { MODEL_ROUTE_STATE_AVAILABLE } else { MODEL_ROUTE_STATE_DEGRADED })
        .bind(if recovered { MODEL_ROUTE_PRIORITY_NORMAL } else { MODEL_ROUTE_PRIORITY_DEMOTED })
        .bind(&now)
        .bind(if recovered { 1 } else { 0 })
        .bind(CACHE_HIT_FAILURE_KIND)
        .bind((!recovered).then_some(next_limit))
        .bind((!recovered).then_some(recovery_limit))
        .bind(hit_rate_percent)
        .execute(&mut *tx)
        .await?;
        event = Some(CacheHitRouteEvent {
            action: UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED,
            result: if recovered {
                "cache_hit_protection_recovered"
            } else {
                "cache_hit_protection_recovering"
            },
            state_before: before_state,
            state_after: if recovered {
                MODEL_ROUTE_STATE_AVAILABLE
            } else {
                MODEL_ROUTE_STATE_DEGRADED
            },
            priority_before: before_priority,
            priority_after: if recovered {
                MODEL_ROUTE_PRIORITY_NORMAL
            } else {
                MODEL_ROUTE_PRIORITY_DEMOTED
            },
            failure_count: 0,
            cooldown_until: None,
            reason_message: None,
            reason_code: Some(CACHE_HIT_FAILURE_KIND),
            failure_kind: None,
        });
    } else {
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET last_seen_at = ?3, cache_last_hit_rate_percent = ?4 WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(model)
        .bind(&now)
        .bind(hit_rate_percent)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let availability_increased = event
        .as_ref()
        .is_some_and(|event| event.action == UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED);
    if let Some(event) = event {
        persist_cache_hit_route_event(pool, account_id, model, event).await?;
    }
    Ok(ModelRouteCacheObservationOutcome {
        observed: true,
        availability_increased,
    })
}

pub(crate) async fn load_model_route_penalties(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    model: Option<&str>,
) -> Result<HashMap<i64, ModelRoutePenalty>> {
    let Some(model) = model.map(str::trim) else {
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
        "SELECT request_model, upstream_request_model, started_at FROM pool_upstream_request_attempts WHERE id = ?1",
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
        .map(|value| value.trim().to_string());
    context.upstream_request_model = context
        .upstream_request_model
        .take()
        .map(|value| value.trim().to_string());
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
    let Some(model) = model.map(str::trim) else {
        return Ok(());
    };
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(());
    }
    let now = now_string();
    let cutoff = cutoff_string();
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, changed_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5) ON CONFLICT(account_id, model) DO UPDATE SET state = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN excluded.state ELSE pool_upstream_account_model_routes.state END, priority = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN excluded.priority ELSE pool_upstream_account_model_routes.priority END, consecutive_failures = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN 0 ELSE pool_upstream_account_model_routes.consecutive_failures END, streak_started_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.streak_started_at END, changed_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN excluded.changed_at ELSE pool_upstream_account_model_routes.changed_at END, last_seen_at = excluded.last_seen_at, last_success_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.last_success_at END, last_failure_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.last_failure_at END, last_failure_kind = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.last_failure_kind END, last_failure_message = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.last_failure_message END, cooldown_until = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.cooldown_until END, reset_fence_at = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.reset_fence_at END, cache_concurrency_limit = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.cache_concurrency_limit END, cache_recovery_limit = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.cache_recovery_limit END, cache_low_hit_streak = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN 0 ELSE pool_upstream_account_model_routes.cache_low_hit_streak END, cache_cooldown_level = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN 0 ELSE pool_upstream_account_model_routes.cache_cooldown_level END, cache_last_hit_rate_percent = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.cache_last_hit_rate_percent END, cache_usage_missing_since = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.cache_usage_missing_since END, cache_usage_missing_reason = CASE WHEN julianday(pool_upstream_account_model_routes.last_seen_at) < julianday(?6) THEN NULL ELSE pool_upstream_account_model_routes.cache_usage_missing_reason END",
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

pub(crate) async fn persist_priority_handoff_event(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: Option<i64>,
    model: &str,
    reason_code: &str,
) -> Result<()> {
    let (action, result) = match reason_code {
        PRIORITY_HANDOFF_SUCCEEDED_REASON => (
            UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED,
            "priority_handoff_succeeded",
        ),
        PRIORITY_HANDOFF_FAILURE_COOLDOWN_REASON => (
            UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_COOLDOWN,
            "priority_handoff_failure_cooldown",
        ),
        PRIORITY_HANDOFF_RECOVERY_PROGRESS_REASON => (
            UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED,
            "priority_handoff_recovery_progress",
        ),
        _ => return Ok(()),
    };
    persist_model_event(
        pool,
        account_id,
        attempt_id,
        model,
        action,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
        result,
        None,
        None,
        None,
        None,
        0,
        None,
        None,
        Some(reason_code),
        None,
        None,
    )
    .await
}

async fn persist_cache_hit_route_event(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: &str,
    event: CacheHitRouteEvent,
) -> Result<()> {
    persist_model_event(
        pool,
        account_id,
        None,
        model,
        event.action,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
        event.result,
        Some(&event.state_before),
        Some(event.state_after),
        Some(&event.priority_before),
        Some(event.priority_after),
        event.failure_count,
        event.cooldown_until.as_deref(),
        event.reason_message,
        event.reason_code,
        None,
        event.failure_kind,
    )
    .await
}

pub(crate) async fn record_model_route_success_from_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: i64,
    request_started_at: Option<&str>,
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
    let cache_protection_enabled =
        resolve_cache_hit_protection_settings(&load_pool_routing_settings(pool).await?).enabled;
    let now = now_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    // Publish the successful terminal fence atomically with model recovery so an
    // older concurrent failure cannot slip through before the outer finalizer.
    sqlx::query("UPDATE pool_upstream_request_attempts SET status = 'success' WHERE id = ?1")
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
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
        return Ok(false);
    };
    if cache_protection_enabled && is_cache_owned_route(&row) {
        // The terminal usage observer decides whether a cache-protected route
        // recovers, remains clamped, or re-enters cooldown. A bare HTTP success
        // is deliberately insufficient while the feature is enabled.
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET last_seen_at = ?3, last_success_at = ?3 WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(&model)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(false);
    }
    if let (Some(started), Some(last_failure)) =
        (request_started_at, row.last_failure_at.as_deref())
        && parse_to_utc_datetime(last_failure).is_some_and(|failure| {
            parse_to_utc_datetime(started).is_some_and(|request| failure > request)
        })
    {
        return Ok(false);
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
        return Ok(false);
    }
    let (before_state, before_priority, _) = effective_row_state(&row, Utc::now());
    let changed = before_state != MODEL_ROUTE_STATE_AVAILABLE
        || before_priority != MODEL_ROUTE_PRIORITY_NORMAL
        || row.consecutive_failures != 0
        || row.cooldown_until.is_some();
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, consecutive_failures = 0, streak_started_at = NULL, changed_at = CASE WHEN ?5 = 1 THEN ?2 ELSE changed_at END, last_seen_at = ?2, last_success_at = ?2, last_failure_at = NULL, last_failure_kind = NULL, last_failure_message = NULL, cooldown_until = NULL, cache_concurrency_limit = NULL, cache_recovery_limit = NULL, cache_low_hit_streak = 0, cache_cooldown_level = 0, cache_usage_missing_since = NULL, cache_usage_missing_reason = NULL WHERE account_id = ?1 AND model = ?6",
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
    Ok(changed)
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

pub(crate) async fn record_temporary_model_route_failure_for_model(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: &str,
    status: StatusCode,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    reason_code: &str,
) -> Result<bool> {
    if !account_is_api_key(load_account_kind(pool, account_id).await?.as_deref()) {
        return Ok(false);
    }
    let model = model.trim();
    record_model_route_failure_inner(
        pool,
        account_id,
        None,
        model,
        status,
        error_message,
        failure_kind,
        Some(Utc::now()),
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
    let Some(model) = attempt_context
        .upstream_request_model
        .or(attempt_context.request_model)
    else {
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
    let AttemptRouteContext {
        request_model: Some(model),
        upstream_request_model,
        started_at,
    } = attempt_context
    else {
        return Ok(false);
    };
    let failure_evidence_model = upstream_request_model.as_deref().unwrap_or(model.as_str());
    if require_explicit_model_failure
        && !is_explicit_model_failure_for_model(status, error_message, Some(failure_evidence_model))
    {
        return Ok(false);
    }
    let attempt_started_at = request_started_at
        .and_then(parse_to_utc_datetime)
        .or_else(|| started_at.as_deref().and_then(parse_to_utc_datetime));
    record_model_route_failure_inner(
        pool,
        account_id,
        Some(attempt_id),
        &model,
        status,
        error_message,
        failure_kind,
        attempt_started_at,
        false,
        reason_code,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "Model failure evidence and concurrency fencing are one state transition."
)]
async fn record_model_route_failure_inner(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempt_id: Option<i64>,
    model: &str,
    status: StatusCode,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    attempt_started_at: Option<DateTime<Utc>>,
    require_explicit_model_failure: bool,
    reason_code: Option<&str>,
) -> Result<bool> {
    if require_explicit_model_failure
        && !is_explicit_model_failure_for_model(status, error_message, Some(model))
    {
        return Ok(false);
    }
    let now = now_string();
    let sanitized_error_message = error_message.and_then(sanitize_account_action_message);
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let existing = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
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
            .bind(model)
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
                        || (started_at == attempt_started_at
                            && attempt_id.is_some_and(|attempt_id| id > attempt_id))
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
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7, ?8, ?9, ?10) ON CONFLICT(account_id, model) DO UPDATE SET state = excluded.state, priority = excluded.priority, consecutive_failures = excluded.consecutive_failures, streak_started_at = excluded.streak_started_at, changed_at = CASE WHEN ?11 = 1 THEN excluded.changed_at ELSE pool_upstream_account_model_routes.changed_at END, last_seen_at = excluded.last_seen_at, last_failure_at = excluded.last_failure_at, last_failure_kind = excluded.last_failure_kind, last_failure_message = excluded.last_failure_message, cooldown_until = excluded.cooldown_until, cache_concurrency_limit = NULL, cache_recovery_limit = NULL, cache_low_hit_streak = 0, cache_cooldown_level = 0, cache_last_hit_rate_percent = NULL, cache_usage_missing_since = NULL, cache_usage_missing_reason = NULL",
    )
    .bind(account_id)
    .bind(model)
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
            attempt_id,
            model,
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
    let Some(row) = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(pool)
    .await? else { return Ok(None) };
    let (before_state, before_priority, _) = effective_row_state(&row, Utc::now());
    let now = now_string();
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET state = ?3, priority = ?4, consecutive_failures = 0, streak_started_at = NULL, changed_at = ?2, reset_fence_at = ?2, last_failure_at = NULL, last_failure_kind = NULL, last_failure_message = NULL, cooldown_until = NULL, cache_concurrency_limit = NULL, cache_recovery_limit = NULL, cache_low_hit_streak = 0, cache_cooldown_level = 0, cache_last_hit_rate_percent = NULL, cache_usage_missing_since = NULL, cache_usage_missing_reason = NULL WHERE account_id = ?1 AND model = ?5",
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
    reset_priority_handoff_for_model(account_id, model);
    let updated = sqlx::query_as::<_, ModelRouteRow>(
        "SELECT account_id, model, state, priority, consecutive_failures, streak_started_at, changed_at, last_seen_at, last_success_at, last_failure_at, last_failure_kind, last_failure_message, cooldown_until, reset_fence_at, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent, cache_usage_missing_since, cache_usage_missing_reason FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .fetch_optional(pool)
    .await?;
    Ok(updated.map(model_state_from_row))
}
