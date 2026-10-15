use super::super::model_health::ModelRouteRow;
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RuntimeModelFact {
    pub(super) before: Option<ModelRouteRow>,
    pub(super) after: ModelRouteRow,
    pub(super) cache_settings_fence: Option<String>,
}

fn canonicalize(row: &mut ModelRouteRow) {
    // The recovery log contains reason codes, never provider error text.
    row.last_failure_message.clone_from(&row.last_failure_kind);
}

impl RuntimeModelFact {
    pub(super) fn apply(&self, current: &mut Option<ModelRouteRow>) {
        if let Some(row) = current.as_mut() {
            canonicalize(row);
        }
        if *current == self.before || current.as_ref() == Some(&self.after) {
            *current = Some(self.after.clone());
        }
    }
}

impl RuntimeRouteFacts {
    fn apply_model_events(
        events: &[RuntimeRouteEvent],
        model: &str,
        row: &mut Option<ModelRouteRow>,
        settings_fence: &str,
        account_fence: &str,
    ) {
        for event in events {
            if event.control_fingerprint != account_fence {
                continue;
            }
            if let Some(fact) = event.model.as_ref().filter(|fact| {
                fact.after.model.eq_ignore_ascii_case(model)
                    && fact
                        .cache_settings_fence
                        .as_deref()
                        .is_none_or(|fence| fence == settings_fence)
            }) {
                fact.apply(row);
            }
        }
    }
}

async fn load_model(
    pool: &Pool<Sqlite>,
    account: i64,
    model: &str,
) -> Result<Option<ModelRouteRow>> {
    sqlx::query_as(
        "SELECT * FROM pool_upstream_account_model_routes WHERE account_id=?1 AND model=?2",
    )
    .bind(account)
    .bind(model)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn fresh(account: i64, model: &str, now: &str) -> ModelRouteRow {
    ModelRouteRow {
        account_id: account,
        model: model.to_string(),
        state: MODEL_ROUTE_STATE_AVAILABLE.to_string(),
        priority: MODEL_ROUTE_PRIORITY_NORMAL.to_string(),
        consecutive_failures: 0,
        streak_started_at: None,
        changed_at: Some(now.to_string()),
        last_seen_at: now.to_string(),
        last_success_at: None,
        last_failure_at: None,
        last_failure_kind: None,
        last_failure_message: None,
        cooldown_until: None,
        reset_fence_at: None,
        cache_concurrency_limit: None,
        cache_recovery_limit: None,
        cache_low_hit_streak: 0,
        cache_cooldown_level: 0,
        cache_last_hit_rate_percent: None,
        cache_usage_missing_since: None,
        cache_usage_missing_reason: None,
    }
}

pub(super) async fn outcome_fact(
    state: &AppState,
    runtime: &RuntimeRouteFacts,
    account: &UpstreamAccountRow,
    model: Option<&str>,
    started: DateTime<Utc>,
    failure: Option<(&str, bool)>,
    now: DateTime<Utc>,
) -> Result<Option<RuntimeModelFact>> {
    if account.kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        return Ok(None);
    }
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return Ok(None);
    };
    let events = runtime.account_events(account.id, true);
    let mut before = load_model(&state.pool, account.id, model).await?;
    let settings_fence = cache_settings_fence(&state.pool).await?;
    RuntimeRouteFacts::apply_model_events(
        &events,
        model,
        &mut before,
        &settings_fence,
        &route_account_control_fingerprint(account),
    );
    if let Some(row) = before.as_mut() {
        canonicalize(row);
    }
    let stamp = format_utc_iso_precise(now);
    let mut after = before
        .clone()
        .unwrap_or_else(|| fresh(account.id, model, &stamp));
    if after
        .reset_fence_at
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .is_some_and(|reset| started <= reset)
    {
        return Ok(None);
    }
    after.last_seen_at.clone_from(&stamp);
    match failure {
        None => {
            let settings = resolve_cache_hit_protection_settings(
                &load_pool_routing_settings(&state.pool).await?,
            );
            let cache_owned = after.last_failure_kind.as_deref() == Some("cache_hit_rate")
                || after.cache_concurrency_limit.is_some()
                || after.cache_recovery_limit.is_some();
            if settings.enabled && cache_owned {
                after.last_success_at = Some(stamp);
            } else if after
                .last_failure_at
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_none_or(|failed| failed <= started)
            {
                let changed = after.state != MODEL_ROUTE_STATE_AVAILABLE
                    || after.priority != MODEL_ROUTE_PRIORITY_NORMAL
                    || after.consecutive_failures != 0
                    || after.cooldown_until.is_some();
                after.state = MODEL_ROUTE_STATE_AVAILABLE.to_string();
                after.priority = MODEL_ROUTE_PRIORITY_NORMAL.to_string();
                after.consecutive_failures = 0;
                after.streak_started_at = None;
                if changed {
                    after.changed_at = Some(stamp.clone());
                }
                after.last_success_at = Some(stamp);
                after.last_failure_at = None;
                after.last_failure_kind = None;
                after.last_failure_message = None;
                after.cooldown_until = None;
                after.cache_concurrency_limit = None;
                after.cache_recovery_limit = None;
                after.cache_low_hit_streak = 0;
                after.cache_cooldown_level = 0;
                after.cache_usage_missing_since = None;
                after.cache_usage_missing_reason = None;
            }
        }
        Some((kind, degrade)) if degrade => {
            // A newer successful request supersedes late failure evidence.
            if after
                .last_success_at
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_some_and(|success| success > started)
            {
                return Ok(None);
            }
            let within = after
                .last_failure_at
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .is_some_and(|last| {
                    now.signed_duration_since(last).num_seconds() <= MODEL_ROUTE_FAILURE_WINDOW_SECS
                });
            after.consecutive_failures = after.consecutive_failures.saturating_add(1);
            let streak = if within {
                after
                    .streak_started_at
                    .clone()
                    .unwrap_or_else(|| stamp.clone())
            } else {
                stamp.clone()
            };
            let age = parse_to_utc_datetime(&streak)
                .map_or(0, |start| now.signed_duration_since(start).num_seconds());
            let cooling = after.consecutive_failures >= MODEL_ROUTE_FAILURE_THRESHOLD
                || age >= MODEL_ROUTE_FAILURE_WINDOW_SECS;
            let cooldown = cooling.then(|| {
                format_utc_iso_precise(
                    now + ChronoDuration::seconds(
                        (MODEL_ROUTE_COOLDOWN_BASE_SECS
                            * (1_i64
                                << after
                                    .consecutive_failures
                                    .saturating_sub(MODEL_ROUTE_FAILURE_THRESHOLD)
                                    .clamp(0, 5) as u32))
                            .min(MODEL_ROUTE_COOLDOWN_MAX_SECS),
                    ),
                )
            });
            let next_state = if cooling {
                MODEL_ROUTE_STATE_COOLING_DOWN
            } else {
                MODEL_ROUTE_STATE_DEGRADED
            };
            let next_priority = if cooling {
                MODEL_ROUTE_PRIORITY_EXCLUDED
            } else {
                MODEL_ROUTE_PRIORITY_DEMOTED
            };
            if after.state != next_state
                || after.priority != next_priority
                || after.cooldown_until != cooldown
            {
                after.changed_at = Some(stamp.clone());
            }
            after.state = next_state.to_string();
            after.priority = next_priority.to_string();
            after.streak_started_at = Some(streak);
            after.last_failure_at = Some(stamp);
            after.last_failure_kind = Some(kind.to_string());
            after.last_failure_message = Some(kind.to_string());
            after.cooldown_until = cooldown;
            after.cache_concurrency_limit = None;
            after.cache_recovery_limit = None;
            after.cache_low_hit_streak = 0;
            after.cache_cooldown_level = 0;
            after.cache_last_hit_rate_percent = None;
            after.cache_usage_missing_since = None;
            after.cache_usage_missing_reason = None;
        }
        Some(_) => {}
    }
    Ok(Some(RuntimeModelFact {
        before,
        after,
        cache_settings_fence: None,
    }))
}

pub(super) async fn materialize(
    conn: &mut SqliteConnection,
    fact: &RuntimeModelFact,
) -> Result<bool> {
    if let Some(fence) = fact.cache_settings_fence.as_deref() {
        let settings: (Option<i64>, Option<i64>, String) = sqlx::query_as("SELECT cache_hit_protection_enabled, cache_hit_low_rate_threshold_percent, updated_at FROM pool_routing_settings WHERE id=1")
            .fetch_one(&mut *conn).await?;
        if fence != format!("{:?}", settings) {
            return Ok(false);
        }
    }
    let mut current: Option<ModelRouteRow> = sqlx::query_as(
        "SELECT * FROM pool_upstream_account_model_routes WHERE account_id=?1 AND model=?2",
    )
    .bind(fact.after.account_id)
    .bind(&fact.after.model)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(row) = current.as_mut() {
        canonicalize(row);
    }
    if current.as_ref() == Some(&fact.after) {
        return Ok(true);
    }
    if current != fact.before {
        return Ok(false);
    }
    // The serialized fields are a fixed typed schema, never caller-controlled SQL.
    let value = serde_json::to_value(&fact.after)?;
    let fields = value.as_object().context("model fact is not an object")?;
    let columns = fields.keys().map(String::as_str).collect::<Vec<_>>();
    let mut query = sqlx::QueryBuilder::<Sqlite>::new(format!(
        "INSERT INTO pool_upstream_account_model_routes ({}) VALUES (",
        columns.join(",")
    ));
    let mut values = query.separated(",");
    for value in fields.values() {
        match value {
            Value::String(value) => {
                values.push_bind(value.clone());
            }
            Value::Number(value) => {
                values.push_bind(value.as_i64().context("invalid model fact number")?);
            }
            Value::Null => {
                values.push_bind(Option::<String>::None);
            }
            _ => bail!("invalid model fact field"),
        }
    }
    query.push(format!(
        ") ON CONFLICT(account_id,model) DO UPDATE SET {}",
        columns
            .iter()
            .filter(|column| !matches!(**column, "account_id" | "model"))
            .map(|column| format!("{column}=excluded.{column}"))
            .collect::<Vec<_>>()
            .join(",")
    ));
    query.build().execute(&mut *conn).await?;
    Ok(true)
}

pub(crate) async fn load_runtime_model_penalties(
    state: &AppState,
    accounts: &[i64],
    model: Option<&str>,
) -> Result<HashMap<i64, ModelRoutePenalty>> {
    let mut penalties = load_model_route_penalties(&state.pool, accounts, model).await?;
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return Ok(penalties);
    };
    let runtime = state.upstream_accounts.route_facts.lock().await.clone();
    let Some(runtime) = runtime else {
        return Ok(penalties);
    };
    let fence = cache_settings_fence(&state.pool).await?;
    for account in accounts {
        let events = runtime.account_events(*account, false);
        let mut row = load_model(&state.pool, *account, model).await?;
        let Some(account_row) = load_upstream_account_row(&state.pool, *account).await? else {
            continue;
        };
        RuntimeRouteFacts::apply_model_events(
            &events,
            model,
            &mut row,
            &fence,
            &route_account_control_fingerprint(&account_row),
        );
        if let Some(row) = row {
            let cooling = row.state == MODEL_ROUTE_STATE_COOLING_DOWN
                && row
                    .cooldown_until
                    .as_deref()
                    .and_then(parse_to_utc_datetime)
                    .is_some_and(|until| until > Utc::now());
            penalties.insert(
                *account,
                if cooling {
                    ModelRoutePenalty::Excluded
                } else if row.state != MODEL_ROUTE_STATE_AVAILABLE {
                    ModelRoutePenalty::Demoted
                } else {
                    ModelRoutePenalty::Normal
                },
            );
        }
    }
    Ok(penalties)
}

async fn cache_settings_fence(pool: &Pool<Sqlite>) -> Result<String> {
    let settings: (Option<i64>, Option<i64>, String) = sqlx::query_as("SELECT cache_hit_protection_enabled, cache_hit_low_rate_threshold_percent, updated_at FROM pool_routing_settings WHERE id=1")
        .fetch_one(pool).await?;
    Ok(format!("{:?}", settings))
}

pub(crate) async fn overlay_runtime_routing_cache(
    state: &AppState,
    cache: &mut PoolRoutingRuntimeCache,
    requested_model: Option<&str>,
) -> Result<()> {
    let runtime = state.upstream_accounts.route_facts.lock().await.clone();
    let Some(runtime) = runtime else {
        return Ok(());
    };
    let keys = {
        let inner = runtime
            .inner
            .lock()
            .map_err(|_| anyhow!("route fact lock poisoned"))?;
        inner
            .pending
            .iter()
            .filter(|pending| pending.event.sequence <= inner.synced_sequence)
            .filter_map(|pending| pending.event.model.as_ref())
            .filter(|fact| {
                requested_model.is_some_and(|model| model.eq_ignore_ascii_case(&fact.after.model))
            })
            .map(|fact| (fact.after.account_id, fact.after.model.clone()))
            .collect::<HashSet<_>>()
    };
    for (account, model) in keys {
        if let Some(row) = projected_model(state, account, Some(&model)).await? {
            cache.model_routing.model_route_runtime.insert(
                (account, model),
                ModelRouteRuntimeSnapshot::from_runtime_row(&row),
            );
        }
    }
    Ok(())
}

async fn projected_model(
    state: &AppState,
    account: i64,
    model: Option<&str>,
) -> Result<Option<ModelRouteRow>> {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return Ok(None);
    };
    let runtime = state.upstream_accounts.route_facts.lock().await.clone();
    let events = runtime
        .as_ref()
        .map_or_else(Vec::new, |runtime| runtime.account_events(account, false));
    let mut row = load_model(&state.pool, account, model).await?;
    if runtime.is_some()
        && let Some(account_row) = load_upstream_account_row(&state.pool, account).await?
    {
        RuntimeRouteFacts::apply_model_events(
            &events,
            model,
            &mut row,
            &cache_settings_fence(&state.pool).await?,
            &route_account_control_fingerprint(&account_row),
        );
    }
    Ok(row)
}

pub(crate) async fn runtime_model_concurrency_limit(
    state: &AppState,
    account: i64,
    model: Option<&str>,
) -> Result<Option<i64>> {
    let Some(row) = projected_model(state, account, model).await? else {
        return Ok(None);
    };
    let expired = row.state == MODEL_ROUTE_STATE_COOLING_DOWN
        && row
            .cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|until| until <= Utc::now());
    if expired {
        return Ok(Some(1));
    }
    Ok(
        resolve_cache_hit_protection_settings(&load_pool_routing_settings(&state.pool).await?)
            .enabled
            .then_some(row.cache_concurrency_limit)
            .flatten()
            .map(|limit| limit.max(1)),
    )
}

pub(crate) async fn runtime_model_requires_expired_probe(
    state: &AppState,
    account: i64,
    model: Option<&str>,
) -> Result<bool> {
    Ok(projected_model(state, account, model)
        .await?
        .is_some_and(|row| {
            row.state == MODEL_ROUTE_STATE_COOLING_DOWN
                && row
                    .cooldown_until
                    .as_deref()
                    .and_then(parse_to_utc_datetime)
                    .is_some_and(|until| until <= Utc::now())
        }))
}

pub(crate) async fn observe_runtime_model_cache(
    state: &AppState,
    account_id: i64,
    requested_model: Option<&str>,
    input: Option<i64>,
    cached: Option<i64>,
    concurrency: i64,
    invoke_id: Option<&str>,
) -> Result<ModelRouteCacheObservationOutcome> {
    let Some(model) = requested_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    else {
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    let runtime = runtime_route_facts(state).await?;
    let gate = runtime.admission_gate.lock().await;
    let Some(account) = load_account_projection(state, Some(&runtime), account_id, true).await?
    else {
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    if account.kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let settings =
        resolve_cache_hit_protection_settings(&load_pool_routing_settings(&state.pool).await?);
    if observations::runtime_request_control_is_stale(state, &account, invoke_id) {
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    if !settings.enabled {
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let fence = cache_settings_fence(&state.pool).await?;
    let events = runtime.account_events(account_id, true);
    let mut before = load_model(&state.pool, account_id, model).await?;
    RuntimeRouteFacts::apply_model_events(
        &events,
        model,
        &mut before,
        &fence,
        &route_account_control_fingerprint(&account),
    );
    if let Some(row) = before.as_mut() {
        canonicalize(row);
    }
    let Some(mut after) = before.clone() else {
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    let now = Utc::now();
    let started =
        observations::runtime_request_started_at(state, account_id, invoke_id).unwrap_or(now);
    if after
        .reset_fence_at
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .is_some_and(|reset| started <= reset)
    {
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let stamp = format_utc_iso_precise(now);
    let active_cooling = after.state == MODEL_ROUTE_STATE_COOLING_DOWN
        && after
            .cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|until| until > now);
    let expired = after.state == MODEL_ROUTE_STATE_COOLING_DOWN
        && after
            .cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|until| until <= now);
    let cache_owned = after.last_failure_kind.as_deref() == Some("cache_hit_rate")
        || after.cache_concurrency_limit.is_some()
        || after.cache_recovery_limit.is_some();
    let mut increased = false;
    let missing = if input.is_none() {
        Some("missing_input_tokens")
    } else if input.is_some_and(|input| input < CACHE_HIT_PROTECTION_MIN_INPUT_TOKENS as i64) {
        Some("input_below_cache_observation_threshold")
    } else if cached.is_none() {
        Some("missing_cache_input_tokens")
    } else {
        None
    };
    after.last_seen_at = stamp.clone();
    if let Some(reason) = missing {
        if !cache_owned {
            return Ok(ModelRouteCacheObservationOutcome::default());
        }
        if !active_cooling {
            after.state = MODEL_ROUTE_STATE_DEGRADED.to_string();
            after.priority = MODEL_ROUTE_PRIORITY_DEMOTED.to_string();
            after.cooldown_until = None;
        }
        if after.cache_concurrency_limit != Some(1)
            || before.as_ref().is_some_and(|row| row.state != after.state)
        {
            after.changed_at = Some(stamp.clone());
        }
        after.cache_recovery_limit = Some(
            after
                .cache_recovery_limit
                .or(after.cache_concurrency_limit)
                .unwrap_or(1),
        );
        after.cache_concurrency_limit = Some(1);
        after.cache_usage_missing_since.get_or_insert(stamp.clone());
        after
            .cache_usage_missing_reason
            .get_or_insert(reason.to_string());
    } else {
        let input = input.context("validated cache input")?;
        let cached = cached.context("validated cache usage")?.clamp(0, input);
        let low = cached.saturating_mul(100)
            < input.saturating_mul(i64::from(settings.low_hit_rate_threshold_percent));
        after.cache_last_hit_rate_percent = Some(cached.saturating_mul(100) / input);
        after.cache_usage_missing_since = None;
        after.cache_usage_missing_reason = None;
        if !active_cooling {
            if low {
                let recovery = after.cache_recovery_limit.unwrap_or(concurrency.max(1));
                let limit = (after.cache_concurrency_limit.unwrap_or(recovery).max(1) / 2).max(1);
                let streak = if limit == 1 {
                    after.cache_low_hit_streak.saturating_add(1)
                } else {
                    0
                };
                let cooling = streak >= 3;
                if cooling {
                    after.cache_cooldown_level =
                        after.cache_cooldown_level.saturating_add(1).clamp(1, 3);
                }
                after.state = if cooling {
                    MODEL_ROUTE_STATE_COOLING_DOWN
                } else {
                    MODEL_ROUTE_STATE_DEGRADED
                }
                .to_string();
                after.priority = if cooling {
                    MODEL_ROUTE_PRIORITY_EXCLUDED
                } else {
                    MODEL_ROUTE_PRIORITY_DEMOTED
                }
                .to_string();
                after.changed_at = Some(stamp.clone());
                after.last_failure_at = Some(stamp.clone());
                after.last_failure_kind = Some("cache_hit_rate".to_string());
                after.last_failure_message = after.last_failure_kind.clone();
                after.cooldown_until = cooling.then(|| {
                    format_utc_iso_precise(
                        now + ChronoDuration::seconds(
                            15 * (1_i64 << (after.cache_cooldown_level - 1) as u32),
                        ),
                    )
                });
                after.cache_concurrency_limit = Some(limit);
                after.cache_recovery_limit = Some(recovery);
                after.cache_low_hit_streak = if cooling { 0 } else { streak };
            } else if expired || after.cache_concurrency_limit.is_some() {
                let recovery = after
                    .cache_recovery_limit
                    .or(after.cache_concurrency_limit)
                    .unwrap_or(1)
                    .max(1);
                let next = after.cache_concurrency_limit.unwrap_or(1).saturating_add(1);
                let recovered = expired || next >= recovery;
                after.state = if recovered {
                    MODEL_ROUTE_STATE_AVAILABLE
                } else {
                    MODEL_ROUTE_STATE_DEGRADED
                }
                .to_string();
                after.priority = if recovered {
                    MODEL_ROUTE_PRIORITY_NORMAL
                } else {
                    MODEL_ROUTE_PRIORITY_DEMOTED
                }
                .to_string();
                after.changed_at = Some(stamp.clone());
                after.cooldown_until = None;
                after.cache_concurrency_limit = (!recovered).then_some(next);
                after.cache_recovery_limit = (!recovered).then_some(recovery);
                after.cache_low_hit_streak = 0;
                after.cache_cooldown_level = 0;
                if recovered
                    && (expired || after.last_failure_kind.as_deref() == Some("cache_hit_rate"))
                {
                    after.last_failure_at = None;
                    after.last_failure_kind = None;
                    after.last_failure_message = None;
                }
                increased = true;
            }
        }
    }
    let health = RuntimeAccountHealth::from_row(&account);
    let event = RuntimeRouteEvent {
        sequence: 0,
        event_id: String::new(),
        account_id,
        control_fingerprint: route_account_control_fingerprint(&account),
        request_started_at: format_utc_iso_precise(started),
        observed_at: stamp,
        health_before: health,
        health_after: None,
        capabilities: Vec::new(),
        sticky: None,
        invoke_id: None,
        reason_code: None,
        action: None,
        update_latest_action: false,
        http_status: None,
        unsupported_model: None,
        model: Some(RuntimeModelFact {
            before,
            after,
            cache_settings_fence: Some(fence),
        }),
        memory_error: None,
    };
    let ack = runtime.append(event)?;
    drop(gate);
    observations::confirm(state, &runtime, ack).await?;
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
    if increased
        && account.enabled != 0
        && account.status == UPSTREAM_ACCOUNT_STATUS_ACTIVE
        && account
            .cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_none_or(|until| until <= Utc::now())
    {
        publish_pool_routing_availability(state);
    }
    Ok(ModelRouteCacheObservationOutcome {
        observed: true,
        availability_increased: increased,
    })
}
