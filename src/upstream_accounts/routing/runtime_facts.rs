use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, sync::Mutex as StdMutex};
use tokio::sync::{Notify, oneshot};

mod audit;
mod journal;
mod model;
mod observations;
mod sticky;
use journal::RouteJournal;
pub(crate) use model::{
    load_runtime_model_penalties, observe_runtime_model_cache, overlay_runtime_routing_cache,
    runtime_model_concurrency_limit, runtime_model_requires_expired_probe,
};
pub(crate) use observations::*;

const MAX_PENDING_EVENTS: usize = 4_096;
const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;
const MATERIALIZATION_WINDOW: Duration = Duration::from_millis(20);
const MAX_BATCH_EVENTS: usize = 32;
const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RuntimeAccountHealth {
    pub(crate) status: String,
    pub(crate) last_error_at: Option<String>,
    pub(crate) last_route_failure_at: Option<String>,
    pub(crate) last_route_failure_kind: Option<String>,
    pub(crate) cooldown_until: Option<String>,
    pub(crate) consecutive_route_failures: i64,
    pub(crate) temporary_route_failure_streak_started_at: Option<String>,
}

impl RuntimeAccountHealth {
    pub(crate) fn from_row(row: &UpstreamAccountRow) -> Self {
        Self {
            status: row.status.clone(),
            last_error_at: row.last_error_at.clone(),
            last_route_failure_at: row.last_route_failure_at.clone(),
            last_route_failure_kind: row.last_route_failure_kind.clone(),
            cooldown_until: row.cooldown_until.clone(),
            consecutive_route_failures: row.consecutive_route_failures,
            temporary_route_failure_streak_started_at: row
                .temporary_route_failure_streak_started_at
                .clone(),
        }
    }

    fn apply(&self, row: &mut UpstreamAccountRow, message: Option<&str>) {
        row.status.clone_from(&self.status);
        row.last_error = message.map(ToOwned::to_owned);
        row.last_error_at.clone_from(&self.last_error_at);
        row.last_route_failure_at
            .clone_from(&self.last_route_failure_at);
        row.last_route_failure_kind
            .clone_from(&self.last_route_failure_kind);
        row.cooldown_until.clone_from(&self.cooldown_until);
        row.consecutive_route_failures = self.consecutive_route_failures;
        row.temporary_route_failure_streak_started_at
            .clone_from(&self.temporary_route_failure_streak_started_at);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RuntimeCapabilityFact {
    pub(crate) axis: UpstreamCapabilityAxis,
    pub(crate) support: String,
    pub(crate) observed_at: String,
    pub(crate) reason: String,
}

impl RuntimeCapabilityFact {
    fn apply(&self, row: &mut UpstreamAccountRow) {
        let (support, observed_at, reason) = match self.axis {
            UpstreamCapabilityAxis::ResponseEndpoint => (
                &mut row.response_endpoint_capability,
                &mut row.response_endpoint_capability_observed_at,
                &mut row.response_endpoint_capability_reason,
            ),
            UpstreamCapabilityAxis::ChatCompletionsEndpoint => (
                &mut row.chat_completions_capability,
                &mut row.chat_completions_capability_observed_at,
                &mut row.chat_completions_capability_reason,
            ),
            UpstreamCapabilityAxis::ImageEndpoint => (
                &mut row.image_endpoint_capability,
                &mut row.image_endpoint_capability_observed_at,
                &mut row.image_endpoint_capability_reason,
            ),
            UpstreamCapabilityAxis::ResponseImageTool => (
                &mut row.response_image_tool_capability,
                &mut row.response_image_tool_capability_observed_at,
                &mut row.response_image_tool_capability_reason,
            ),
            UpstreamCapabilityAxis::CodexImagegen => (
                &mut row.codex_imagegen_capability,
                &mut row.codex_imagegen_capability_observed_at,
                &mut row.codex_imagegen_capability_reason,
            ),
            UpstreamCapabilityAxis::StandaloneSearch => (
                &mut row.standalone_search_capability,
                &mut row.standalone_search_capability_observed_at,
                &mut row.standalone_search_capability_reason,
            ),
        };
        if observed_at
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .zip(parse_to_utc_datetime(&self.observed_at))
            .is_some_and(|(old, new)| old > new)
        {
            return;
        }
        *support = Some(self.support.clone());
        *observed_at = Some(self.observed_at.clone());
        *reason = Some(self.reason.clone());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RuntimeStickyFact {
    pub(crate) sticky_key: String,
    pub(crate) model_key: Option<String>,
    pub(crate) generation_before: i64,
    pub(crate) generation_after: i64,
    pub(crate) account_id: Option<i64>,
    pub(crate) owner_before: Option<i64>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) attempt_public_id: Option<String>,
    pub(crate) routing_source: Option<String>,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RuntimeRouteEvent {
    pub(crate) sequence: u64,
    pub(crate) event_id: String,
    pub(crate) account_id: i64,
    pub(crate) control_fingerprint: String,
    pub(crate) request_started_at: String,
    pub(crate) observed_at: String,
    pub(crate) health_before: RuntimeAccountHealth,
    pub(crate) health_after: Option<RuntimeAccountHealth>,
    pub(crate) capabilities: Vec<RuntimeCapabilityFact>,
    pub(crate) sticky: Option<RuntimeStickyFact>,
    pub(crate) invoke_id: Option<String>,
    pub(crate) reason_code: Option<String>,
    pub(crate) action: Option<String>,
    #[serde(default)]
    pub(crate) update_latest_action: bool,
    pub(crate) http_status: Option<i64>,
    pub(crate) unsupported_model: Option<String>,
    model: Option<model::RuntimeModelFact>,
    #[serde(skip)]
    pub(crate) memory_error: Option<String>,
}

impl RuntimeRouteEvent {
    fn memory_bytes(&self) -> usize {
        serde_json::to_vec(self)
            .map_or(MAX_PENDING_BYTES + 1, |bytes| bytes.len())
            .saturating_add(self.memory_error.as_ref().map_or(0, String::capacity))
            .saturating_add(std::mem::size_of::<PendingRouteEvent>())
    }

    fn message(&self) -> Option<&str> {
        self.memory_error.as_deref().or(self.reason_code.as_deref())
    }

    fn apply_account(&self, row: &mut UpstreamAccountRow) -> bool {
        if self.control_fingerprint != route_account_control_fingerprint(row) {
            return false;
        }
        if let Some(health) = self.health_after.as_ref() {
            let current = RuntimeAccountHealth::from_row(row);
            if current == self.health_before || current == *health {
                health.apply(row, self.message());
            }
        }
        for capability in &self.capabilities {
            capability.apply(row);
        }
        if self.update_latest_action
            && let Some(action) = self.action.as_ref()
            && row
                .last_action_at
                .as_deref()
                .and_then(parse_to_utc_datetime)
                .zip(parse_to_utc_datetime(&self.observed_at))
                .is_none_or(|(old, new)| old <= new)
        {
            row.last_action = Some(action.clone());
            row.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
            row.last_action_reason_code.clone_from(&self.reason_code);
            row.last_action_reason_message = self.message().map(ToOwned::to_owned);
            row.last_action_http_status = self.http_status;
            row.last_action_invoke_id.clone_from(&self.invoke_id);
            row.last_action_at = Some(self.observed_at.clone());
        }
        true
    }
}

// Informational account edits do not change this fence; credential replacement,
// enable/disable and routing policy changes do. Only its digest enters recovery.
pub(crate) fn route_account_control_fingerprint(row: &UpstreamAccountRow) -> String {
    let mut hash = Sha256::new();
    hash.update(row.runtime_routing_generation.to_le_bytes());
    hash.update(format!(
        "{:?}",
        (
            &row.kind,
            row.enabled,
            &row.encrypted_credentials,
            &row.upstream_base_url,
            &row.group_name,
            &row.policy_response_endpoint_capability_override,
            &row.policy_chat_completions_capability_override,
            &row.policy_image_endpoint_capability_override,
            &row.policy_response_image_tool_capability_override,
            &row.policy_codex_imagegen_capability_override,
            &row.policy_standalone_search_capability_override
        )
    ));
    hash.update(format!(
        "{:?}",
        (
            row.policy_status_change_upstream_http_401,
            row.policy_status_change_upstream_http_402,
            row.policy_status_change_upstream_http_403,
            row.policy_status_change_reauth_required,
            row.policy_status_change_upstream_http_429_rate_limit,
            row.policy_status_change_upstream_http_429_quota_exhausted,
            row.policy_status_change_transport_failure,
            row.policy_status_change_upstream_server_overloaded,
            row.policy_status_change_upstream_http_5xx
        )
    ));
    format!("{:x}", hash.finalize())
}

#[derive(Debug)]
struct PendingRouteEvent {
    event: RuntimeRouteEvent,
    bytes: usize,
    admission: Option<oneshot::Sender<std::result::Result<(), String>>>,
}

#[derive(Debug)]
struct RouteFactsInner {
    journal: RouteJournal,
    pending: VecDeque<PendingRouteEvent>,
    pending_bytes: usize,
    synced_sequence: u64,
    failure: Option<String>,
    rejected: u64,
    replayed: u64,
    last_materialization_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeRouteHealth {
    pub(crate) pending_events: usize,
    pub(crate) pending_bytes: usize,
    pub(crate) oldest_age_ms: u64,
    pub(crate) rejected: u64,
    pub(crate) replayed: u64,
    pub(crate) last_materialization_ms: u64,
    pub(crate) failed: bool,
}

#[derive(Debug)]
pub(crate) struct RuntimeRouteFacts {
    inner: StdMutex<RouteFactsInner>,
    pub(crate) admission_gate: Mutex<()>,
    materialization_gate: Mutex<()>,
    changed: Notify,
    stop: CancellationToken,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl RuntimeRouteFacts {
    #[cfg(test)]
    pub(crate) async fn pause_materialization_for_test(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.materialization_gate.lock().await
    }
    #[cfg(test)]
    pub(crate) async fn flush_for_test(&self, pool: &Pool<Sqlite>) -> Result<()> {
        self.materialize_batch(pool).await
    }
    pub(crate) fn open(state: &AppState) -> Result<Arc<Self>> {
        #[cfg(not(test))]
        let directory =
            crate::terminal_journal::terminal_journal_directory(&state.config.database_path);
        #[cfg(test)]
        let directory = state.config.proxy_raw_dir.join("route-journal");
        let (journal, events) = RouteJournal::open(&directory, &state.oauth_installation_seed)?;
        let synced_sequence = events.last().map_or(0, |event| event.sequence);
        let pending_bytes = events
            .iter()
            .map(RuntimeRouteEvent::memory_bytes)
            .sum::<usize>();
        if events.len() > MAX_PENDING_EVENTS || pending_bytes > MAX_PENDING_BYTES {
            bail!("route recovery exceeds bounded projection capacity; journal preserved");
        }
        let runtime = Arc::new(Self {
            inner: StdMutex::new(RouteFactsInner {
                journal,
                replayed: events.len() as u64,
                pending: events
                    .into_iter()
                    .map(|event| {
                        let bytes = event.memory_bytes();
                        PendingRouteEvent {
                            event,
                            bytes,
                            admission: None,
                        }
                    })
                    .collect(),
                pending_bytes,
                synced_sequence,
                failure: None,
                rejected: 0,
                last_materialization_ms: 0,
            }),
            admission_gate: Mutex::new(()),
            materialization_gate: Mutex::new(()),
            changed: Notify::new(),
            stop: CancellationToken::new(),
            workers: Mutex::new(Vec::new()),
        });
        let sync_runtime = runtime.clone();
        let sync_shutdown = runtime.stop.clone();
        let sync_handle = crate::observability::diagnostics::spawn(async move {
            let mut interval = tokio::time::interval(MATERIALIZATION_WINDOW);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { _ = interval.tick() => {}, _ = sync_shutdown.cancelled() => { sync_runtime.sync_pending(); break; } }
                sync_runtime.sync_pending();
            }
        });
        let materializer = runtime.clone();
        let pool = state.pool.clone();
        let shutdown = runtime.stop.clone();
        let materialization_handle = crate::observability::diagnostics::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(MATERIALIZATION_WINDOW) => {},
                    _ = materializer.changed.notified() => {},
                    _ = shutdown.cancelled() => break,
                }
                let result = tokio::select! {
                    result = materializer.materialize_batch(&pool) => result,
                    _ = shutdown.cancelled() => break,
                };
                if let Err(error) = result {
                    warn!(error = %error, "route fact materialization retained for retry");
                    tokio::select! { _ = tokio::time::sleep(Duration::from_millis(100)) => {}, _ = shutdown.cancelled() => break }
                }
            }
        });
        *runtime.workers.try_lock().expect("new worker slot") =
            vec![sync_handle, materialization_handle];
        Ok(runtime)
    }

    pub(crate) async fn shutdown_and_sync(&self) {
        self.stop.cancel();
        self.sync_pending();
        let handles = std::mem::take(&mut *self.workers.lock().await);
        for handle in handles {
            if let Err(error) = handle.await {
                warn!(%error, "route fact worker terminated");
            }
        }
    }

    pub(crate) fn health_snapshot(&self) -> RuntimeRouteHealth {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        RuntimeRouteHealth {
            pending_events: inner.pending.len(),
            pending_bytes: inner.pending_bytes,
            oldest_age_ms: inner
                .pending
                .front()
                .and_then(|pending| parse_to_utc_datetime(&pending.event.observed_at))
                .map_or(0, |at| {
                    Utc::now()
                        .signed_duration_since(at)
                        .num_milliseconds()
                        .max(0) as u64
                }),
            rejected: inner.rejected,
            replayed: inner.replayed,
            last_materialization_ms: inner.last_materialization_ms,
            failed: inner.failure.is_some(),
        }
    }

    // The producer gate serializes reduction, not fsync or SQLite materialization.
    pub(crate) fn append(
        &self,
        mut event: RuntimeRouteEvent,
    ) -> Result<oneshot::Receiver<std::result::Result<(), String>>> {
        if self.stop.is_cancelled() {
            bail!("route fact admission closed during shutdown");
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("route fact lock poisoned"))?;
        if let Some(failure) = inner.failure.as_ref() {
            bail!("route journal unavailable: {failure}");
        }
        event.event_id = nanoid::nanoid!();
        let bytes = event.memory_bytes();
        if inner.pending.len() >= MAX_PENDING_EVENTS
            || inner.pending_bytes.saturating_add(bytes) > MAX_PENDING_BYTES
        {
            inner.rejected = inner.rejected.saturating_add(1);
            bail!("route observation capacity exhausted; observation not accepted");
        }
        if let Err(error) = inner.journal.append(&mut event) {
            inner.rejected = inner.rejected.saturating_add(1);
            if inner.journal.append_failed() {
                inner.failure = Some(error.to_string());
            }
            return Err(error);
        }
        let (sender, receiver) = oneshot::channel();
        inner.pending.push_back(PendingRouteEvent {
            event,
            bytes,
            admission: Some(sender),
        });
        inner.pending_bytes += bytes;
        Ok(receiver)
    }

    fn sync_pending(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if !inner.pending.iter().any(|event| event.admission.is_some()) {
            return;
        }
        let result = inner.journal.sync().map_err(|error| error.to_string());
        if let Err(error) = result.as_ref() {
            inner.failure = Some(error.clone());
        } else if let Some(sequence) = inner.pending.back().map(|event| event.event.sequence) {
            inner.synced_sequence = sequence;
        }
        for pending in &mut inner.pending {
            if let Some(sender) = pending.admission.take() {
                let _ = sender.send(result.clone());
            }
        }
        drop(inner);
        self.changed.notify_one();
    }

    fn account_events(&self, id: i64, unconfirmed: bool) -> Vec<RuntimeRouteEvent> {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner
            .pending
            .iter()
            .filter(|pending| {
                pending.event.account_id == id
                    && (unconfirmed || pending.event.sequence <= inner.synced_sequence)
            })
            .map(|pending| pending.event.clone())
            .collect()
    }

    pub(crate) fn apply_rule(&self, row: &UpstreamAccountRow, rule: &mut EffectiveRoutingRule) {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let fence = route_account_control_fingerprint(row);
        for pending in &inner.pending {
            if pending.event.account_id == row.id
                && pending.event.sequence <= inner.synced_sequence
                && pending.event.control_fingerprint == fence
                && let Some(model) = pending.event.unsupported_model.as_ref()
                && !rule.system_denied_models.contains(model)
            {
                rule.system_denied_models.push(model.clone());
            }
        }
    }

    fn sticky_events(&self, key: &str, include_unconfirmed: bool) -> Vec<RuntimeRouteEvent> {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner
            .pending
            .iter()
            .filter(|pending| {
                (include_unconfirmed || pending.event.sequence <= inner.synced_sequence)
                    && pending
                        .event
                        .sticky
                        .as_ref()
                        .is_some_and(|sticky| sticky.sticky_key == key)
            })
            .map(|pending| pending.event.clone())
            .collect()
    }

    async fn materialize_batch(&self, pool: &Pool<Sqlite>) -> Result<()> {
        let _batch_gate = self.materialization_gate.lock().await;
        let batch = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| anyhow!("route fact lock poisoned"))?;
            let mut bytes = 0;
            inner
                .pending
                .iter()
                .take_while(|pending| {
                    bytes += pending.bytes;
                    pending.event.sequence <= inner.synced_sequence && bytes <= MAX_BATCH_BYTES
                })
                .take(MAX_BATCH_EVENTS)
                .map(|pending| pending.event.clone())
                .collect::<Vec<_>>()
        };
        if batch.is_empty() {
            return Ok(());
        }
        let _permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
            .await;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        for event in &batch {
            materialize_route_event(tx.as_mut(), event).await?;
        }
        tx.commit().await?;
        let sequence = batch.last().expect("nonempty route batch").sequence;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("route fact lock poisoned"))?;
        inner.journal.acknowledge(sequence)?;
        inner.last_materialization_ms = batch
            .first()
            .and_then(|event| parse_to_utc_datetime(&event.observed_at))
            .map_or(0, |at| {
                Utc::now()
                    .signed_duration_since(at)
                    .num_milliseconds()
                    .max(0) as u64
            });
        while inner
            .pending
            .front()
            .is_some_and(|pending| pending.event.sequence <= sequence)
        {
            if let Some(pending) = inner.pending.pop_front() {
                inner.pending_bytes = inner.pending_bytes.saturating_sub(pending.bytes);
            }
        }
        Ok(())
    }
}

pub(crate) async fn runtime_route_facts(state: &AppState) -> Result<Arc<RuntimeRouteFacts>> {
    let mut slot = state.upstream_accounts.route_facts.lock().await;
    if let Some(runtime) = slot.as_ref() {
        return Ok(runtime.clone());
    }
    let runtime = RuntimeRouteFacts::open(state)?;
    *slot = Some(runtime.clone());
    Ok(runtime)
}

pub(crate) async fn load_runtime_route_account(
    state: &AppState,
    account_id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    let runtime = state.upstream_accounts.route_facts.lock().await.clone();
    load_account_projection(state, runtime.as_deref(), account_id, false).await
}

async fn load_account_projection(
    state: &AppState,
    runtime: Option<&RuntimeRouteFacts>,
    account_id: i64,
    unconfirmed: bool,
) -> Result<Option<UpstreamAccountRow>> {
    let events = runtime.map_or_else(Vec::new, |runtime| {
        runtime.account_events(account_id, unconfirmed)
    });
    let mut row = load_upstream_account_row(&state.pool, account_id).await?;
    if let Some(row) = row.as_mut() {
        for event in events {
            event.apply_account(row);
        }
    }
    Ok(row)
}

pub(crate) async fn load_runtime_effective_rule(
    state: &AppState,
    account: i64,
) -> Result<EffectiveRoutingRule> {
    let mut rule = load_effective_routing_rule_for_account(&state.pool, account).await?;
    if let Some(row) = load_upstream_account_row(&state.pool, account).await?
        && let Some(runtime) = state.upstream_accounts.route_facts.lock().await.as_ref()
    {
        runtime.apply_rule(&row, &mut rule);
    }
    Ok(rule)
}

pub(crate) async fn load_runtime_effective_rules(
    state: &AppState,
    accounts: &[i64],
) -> Result<HashMap<i64, EffectiveRoutingRule>> {
    let mut rules = load_effective_routing_rules_for_accounts(&state.pool, accounts).await?;
    let runtime = state.upstream_accounts.route_facts.lock().await.clone();
    if let Some(runtime) = runtime {
        for (id, rule) in &mut rules {
            if let Some(row) = load_upstream_account_row(&state.pool, *id).await? {
                runtime.apply_rule(&row, rule);
            }
        }
    }
    Ok(rules)
}

pub(crate) async fn load_runtime_sticky_route(
    state: &AppState,
    key: &str,
    model: Option<&str>,
) -> Result<(Option<PoolStickyRouteRow>, i64)> {
    let runtime = state.upstream_accounts.route_facts.lock().await.clone();
    let (route, generation, _) =
        sticky::load_projection(state, runtime.as_deref(), key, model, false).await?;
    Ok((route, generation))
}

async fn materialize_route_event(
    conn: &mut SqliteConnection,
    event: &RuntimeRouteEvent,
) -> Result<()> {
    let Some(mut account) = load_upstream_account_row_conn(conn, event.account_id).await? else {
        return Ok(());
    };
    if !event.apply_account(&mut account) {
        return Ok(());
    }
    if let Some(model) = event.unsupported_model.as_ref()
        && let Some(key) = unsupported_model_system_tag_key(model)
    {
        ensure_protected_system_tag_conn(conn, &unsupported_model_system_tag_name(model), &key)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO pool_upstream_account_tags(account_id,tag_id,created_at,updated_at) SELECT ?1,id,?2,?2 FROM pool_tags WHERE system_key=?3")
            .bind(event.account_id).bind(&event.observed_at).bind(key).execute(&mut *conn).await?;
    }
    if let Some(health) = event.health_after.as_ref()
        && RuntimeAccountHealth::from_row(&account) == *health
    {
        sqlx::query("UPDATE pool_upstream_accounts SET status=?2, last_error=?3, last_error_at=?4, last_route_failure_at=?5, last_route_failure_kind=?6, cooldown_until=?7, consecutive_route_failures=?8, temporary_route_failure_streak_started_at=?9, updated_at=?10 WHERE id=?1")
            .bind(event.account_id).bind(&health.status).bind(event.message()).bind(&health.last_error_at)
            .bind(&health.last_route_failure_at).bind(&health.last_route_failure_kind).bind(&health.cooldown_until)
            .bind(health.consecutive_route_failures).bind(&health.temporary_route_failure_streak_started_at).bind(if parse_to_utc_datetime(&account.updated_at) > parse_to_utc_datetime(&event.observed_at) { &account.updated_at } else { &event.observed_at })
            .execute(&mut *conn).await?;
    }
    for capability in &event.capabilities {
        let query = format!(
            "UPDATE pool_upstream_accounts SET {}=?2, {}=?3, {}=?4 WHERE id=?1 AND ({} IS NULL OR julianday({}) <= julianday(?3))",
            capability.axis.observed_column(),
            capability.axis.observed_at_column(),
            capability.axis.reason_column(),
            capability.axis.observed_at_column(),
            capability.axis.observed_at_column()
        );
        sqlx::query(&query)
            .bind(event.account_id)
            .bind(&capability.support)
            .bind(&capability.observed_at)
            .bind(&capability.reason)
            .execute(&mut *conn)
            .await?;
    }
    let model_applied = if let Some(fact) = event.model.as_ref() {
        model::materialize(conn, fact).await?
    } else {
        false
    };
    if let Some(sticky) = event.sticky.as_ref() {
        sticky::materialize(conn, event, sticky, &account).await?;
    }
    audit::materialize(conn, event, &account, model_applied).await?;
    Ok(())
}
