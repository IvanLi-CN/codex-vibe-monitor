use super::*;

pub(crate) const ANALYTICS_READ_PAGE_MAX_ROWS: usize = ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH;
pub(crate) const ANALYTICS_READ_PAGE_MAX_ENCODED_BYTES: usize =
    ANALYTICS_REPLICA_MAX_ENCODED_BATCH_BYTES;
pub(crate) const ANALYTICS_READ_MAX_TAIL_ROWS: usize = 200_000;
pub(crate) const ANALYTICS_READ_MAX_TAIL_ENCODED_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const ANALYTICS_READ_DEADLINE: Duration = Duration::from_secs(15);
pub(crate) const ANALYTICS_READ_MAX_STALE: Duration = Duration::from_secs(15);

const ANALYTICS_FACT_SELECT_SQL: &str = r#"
    SELECT fact_version, source, source_row_id, invoke_id,
           to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS occurred_at,
           occurred_at_epoch_ms,
           stats_date::text AS stats_date, model, request_model, response_model,
           reasoning_effort, upstream_account_id, upstream_account_name, status,
           success_like, terminal, failure_kind, failure_class, is_actionable,
           input_tokens, output_tokens, cache_input_tokens, reasoning_tokens,
           total_tokens, cost::double precision AS cost,
           cost_input::double precision AS cost_input,
           cost_cache_write::double precision AS cost_cache_write,
           cost_cache_read::double precision AS cost_cache_read,
           cost_output::double precision AS cost_output,
           cost_reasoning::double precision AS cost_reasoning,
           t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
           t_upstream_ttfb_ms, t_upstream_stream_ms, first_token_ms, t_resp_parse_ms
    FROM cvm_analytics.analytics_facts
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AnalyticsFactScope {
    All,
    ProxyOnly,
    Account(i64),
    AccountProxyOnly(i64),
}

impl AnalyticsFactScope {
    fn validate(self) -> AnalyticsReadResult<Self> {
        match self {
            Self::Account(account_id) | Self::AccountProxyOnly(account_id) if account_id <= 0 => {
                return Err(AnalyticsReadError::InvalidQuery(
                    "upstream account id must be positive".to_string(),
                ));
            }
            _ => {}
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsFactKey {
    pub(crate) stats_date: String,
    pub(crate) occurred_at_epoch_ms: i64,
    pub(crate) source: String,
    pub(crate) source_row_id: i64,
}

impl AnalyticsFactKey {
    fn from_fact(fact: &AnalyticsFact) -> AnalyticsReadResult<Self> {
        if fact.source.trim().is_empty() || fact.invoke_id.trim().is_empty() {
            return Err(invalid_fact("fact identity is incomplete"));
        }
        if fact.source_row_id <= 0 {
            return Err(invalid_fact("fact source row id must be positive"));
        }
        NaiveDate::parse_from_str(&fact.stats_date, "%Y-%m-%d")
            .map_err(|_| invalid_fact("fact stats date is invalid"))?;
        Ok(Self {
            stats_date: fact.stats_date.clone(),
            occurred_at_epoch_ms: fact.occurred_at_epoch_ms,
            source: fact.source.clone(),
            source_row_id: fact.source_row_id,
        })
    }

    fn parse_date(&self) -> AnalyticsReadResult<NaiveDate> {
        NaiveDate::parse_from_str(&self.stats_date, "%Y-%m-%d")
            .map_err(|_| invalid_fact("fact cursor stats date is invalid"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsReadFenceIdentity {
    pub(crate) generation: i64,
    pub(crate) baseline_generation: i64,
    pub(crate) coverage_generation: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) coverage_watermark: i64,
    pub(crate) source_checkpoint: i64,
    pub(crate) source_acknowledged_through: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsFactCursor {
    pub(crate) scope: AnalyticsFactScope,
    pub(crate) fence: AnalyticsReadFenceIdentity,
    pub(crate) high_watermark: Option<AnalyticsFactKey>,
    pub(crate) last_key: Option<AnalyticsFactKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyticsReadFence {
    pub(crate) replica_id: String,
    pub(crate) scope: AnalyticsFactScope,
    pub(crate) identity: AnalyticsReadFenceIdentity,
    pub(crate) high_watermark: Option<AnalyticsFactKey>,
}

impl AnalyticsReadFence {
    pub(crate) fn cursor_after(&self, last_key: Option<AnalyticsFactKey>) -> AnalyticsFactCursor {
        AnalyticsFactCursor {
            scope: self.scope,
            fence: self.identity,
            high_watermark: self.high_watermark.clone(),
            last_key,
        }
    }

    pub(crate) fn validate_cursor(&self, cursor: &AnalyticsFactCursor) -> AnalyticsReadResult<()> {
        if cursor.scope != self.scope
            || cursor.fence != self.identity
            || cursor.high_watermark != self.high_watermark
        {
            return Err(AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
                AnalyticsBackend::Postgres,
                "analytics read cursor fence changed; restart the read",
            )));
        }
        if let (Some(last_key), Some(high_watermark)) =
            (cursor.last_key.as_ref(), self.high_watermark.as_ref())
            && last_key > high_watermark
        {
            return Err(invalid_cursor(
                "analytics read cursor is past its high watermark",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnalyticsFactReadQuery {
    pub(crate) scope: AnalyticsFactScope,
    pub(crate) start_at: Option<DateTime<Utc>>,
    pub(crate) end_at: Option<DateTime<Utc>>,
    pub(crate) source_snapshot_id: Option<i64>,
}

impl AnalyticsFactReadQuery {
    pub(crate) fn for_range(
        scope: AnalyticsFactScope,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
    ) -> AnalyticsReadResult<Self> {
        if start_at >= end_at {
            return Err(AnalyticsReadError::InvalidQuery(
                "analytics fact range must be non-empty".to_string(),
            ));
        }
        scope.validate()?;
        Ok(Self {
            scope,
            start_at: Some(start_at),
            end_at: Some(end_at),
            source_snapshot_id: None,
        })
    }

    pub(crate) fn all(scope: AnalyticsFactScope) -> AnalyticsReadResult<Self> {
        scope.validate()?;
        Ok(Self {
            scope,
            start_at: None,
            end_at: None,
            source_snapshot_id: None,
        })
    }

    pub(crate) fn with_source_snapshot_id(
        mut self,
        source_snapshot_id: i64,
    ) -> AnalyticsReadResult<Self> {
        if source_snapshot_id < 0 {
            return Err(AnalyticsReadError::InvalidQuery(
                "analytics source snapshot id must not be negative".to_string(),
            ));
        }
        self.source_snapshot_id = Some(source_snapshot_id);
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnalyticsFactReadSnapshot {
    pub(crate) fence: AnalyticsReadFence,
    pub(crate) facts: Vec<AnalyticsFact>,
}

#[derive(Debug, FromRow)]
struct AnalyticsFactKeyRow {
    stats_date: String,
    occurred_at_epoch_ms: i64,
    source: String,
    source_row_id: i64,
}

impl AnalyticsFactKeyRow {
    fn into_key(self) -> AnalyticsReadResult<AnalyticsFactKey> {
        let key = AnalyticsFactKey {
            stats_date: self.stats_date,
            occurred_at_epoch_ms: self.occurred_at_epoch_ms,
            source: self.source,
            source_row_id: self.source_row_id,
        };
        key.parse_date()?;
        if key.source.trim().is_empty() || key.source_row_id <= 0 {
            return Err(invalid_fact("analytics fact high watermark is invalid"));
        }
        Ok(key)
    }
}

pub(crate) async fn query_bounded_facts(
    runtime: &PgAnalyticsRuntime,
    query: AnalyticsFactReadQuery,
) -> AnalyticsReadResult<AnalyticsFactReadSnapshot> {
    query.scope.validate()?;
    let started_at = Instant::now();
    let mut transaction =
        tokio::time::timeout(ANALYTICS_READ_DEADLINE, runtime.require_read_transaction())
            .await
            .map_err(|_| read_deadline_exceeded())??;
    ensure_read_deadline(started_at)?;
    let state = tokio::time::timeout(
        remaining_read_deadline(started_at)?,
        load_read_replica_state(&mut transaction),
    )
    .await
    .map_err(|_| read_deadline_exceeded())??;
    let high_watermark = load_fact_high_watermark(&mut transaction, query, started_at).await?;
    let fence = AnalyticsReadFence {
        replica_id: state.replica_id,
        scope: query.scope,
        identity: AnalyticsReadFenceIdentity {
            generation: state.generation,
            baseline_generation: state.baseline_generation,
            coverage_generation: state.coverage_generation,
            coverage_revision: state.coverage_revision,
            coverage_watermark: state.coverage_watermark,
            source_checkpoint: state.consumer_checkpoint,
            source_acknowledged_through: state.source_acknowledged_through,
        },
        high_watermark,
    };

    let mut facts = Vec::new();
    let mut last_key = None;
    let mut total_encoded_bytes = 0_usize;
    loop {
        ensure_read_deadline(started_at)?;
        let page = load_fact_page(
            &mut transaction,
            query,
            last_key.as_ref(),
            fence.high_watermark.as_ref(),
            started_at,
        )
        .await?;
        if page.is_empty() {
            break;
        }
        validate_fact_page(last_key.as_ref(), fence.high_watermark.as_ref(), &page)?;
        let page_encoded_bytes = page
            .iter()
            .map(encoded_fact_size)
            .collect::<AnalyticsReadResult<Vec<_>>>()?
            .into_iter()
            .try_fold(0_usize, |total, size| {
                total
                    .checked_add(size)
                    .ok_or_else(|| invalid_budget("analytics fact page byte budget overflow"))
            })?;
        if page_encoded_bytes > ANALYTICS_READ_PAGE_MAX_ENCODED_BYTES {
            return Err(invalid_budget(
                "analytics fact page exceeds the encoded byte budget",
            ));
        }
        total_encoded_bytes = total_encoded_bytes
            .checked_add(page_encoded_bytes)
            .ok_or_else(|| invalid_budget("analytics fact total byte budget overflow"))?;
        if total_encoded_bytes > ANALYTICS_READ_MAX_TAIL_ENCODED_BYTES {
            return Err(invalid_budget(
                "analytics fact tail exceeds the encoded byte budget",
            ));
        }
        if facts.len().saturating_add(page.len()) > ANALYTICS_READ_MAX_TAIL_ROWS {
            return Err(invalid_budget("analytics fact tail exceeds the row budget"));
        }
        last_key = page.last().map(AnalyticsFactKey::from_fact).transpose()?;
        facts.extend(page);
        ensure_tail_budget_complete(
            facts.len(),
            last_key.as_ref(),
            fence.high_watermark.as_ref(),
        )?;
        if last_key == fence.high_watermark {
            break;
        }
    }
    let remaining = remaining_read_deadline(started_at)?;
    tokio::time::timeout(remaining, transaction.commit())
        .await
        .map_err(|_| read_deadline_exceeded())?
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to commit PostgreSQL analytics bounded fact read: {error}"
            ))
        })?;
    Ok(AnalyticsFactReadSnapshot { fence, facts })
}

async fn load_read_replica_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AnalyticsReadResult<AnalyticsReplicaState> {
    sqlx::query_as::<_, AnalyticsReplicaState>(
        r#"
        SELECT replica_id, status, generation, baseline_generation,
               baseline_change_id, coverage_generation, coverage_revision,
               coverage_watermark, consumer_checkpoint, source_acknowledged_through,
               service_eligible, fact_date_semantics_version,
               last_error_code,
               to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at
        FROM cvm_analytics.analytics_replica_state
        WHERE id = 1
        "#,
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to load PostgreSQL analytics read fence: {error}"
        ))
    })
}

async fn load_fact_high_watermark(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    query: AnalyticsFactReadQuery,
    started_at: Instant,
) -> AnalyticsReadResult<Option<AnalyticsFactKey>> {
    let mut builder = QueryBuilder::<sqlx::Postgres>::new(
        "SELECT stats_date::text AS stats_date, occurred_at_epoch_ms, source, source_row_id FROM cvm_analytics.analytics_facts WHERE 1 = 1",
    );
    append_fact_filters(&mut builder, query, None, None);
    builder.push(" ORDER BY stats_date DESC, occurred_at_epoch_ms DESC, source DESC, source_row_id DESC LIMIT 1");
    let remaining = remaining_read_deadline(started_at)?;
    let row = tokio::time::timeout(
        remaining,
        builder
            .build_query_as::<AnalyticsFactKeyRow>()
            .fetch_optional(&mut **transaction),
    )
    .await
    .map_err(|_| read_deadline_exceeded())?
    .map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to capture PostgreSQL analytics fact high watermark: {error}"
        ))
    })?;
    row.map(AnalyticsFactKeyRow::into_key).transpose()
}

async fn load_fact_page(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    query: AnalyticsFactReadQuery,
    after_key: Option<&AnalyticsFactKey>,
    high_watermark: Option<&AnalyticsFactKey>,
    started_at: Instant,
) -> AnalyticsReadResult<Vec<AnalyticsFact>> {
    let mut builder = QueryBuilder::<sqlx::Postgres>::new(ANALYTICS_FACT_SELECT_SQL);
    builder.push(" WHERE 1 = 1");
    append_fact_filters(&mut builder, query, after_key, high_watermark);
    builder.push(
        " ORDER BY stats_date ASC, occurred_at_epoch_ms ASC, source ASC, source_row_id ASC LIMIT ",
    );
    builder.push_bind(ANALYTICS_READ_PAGE_MAX_ROWS as i64);
    let remaining = remaining_read_deadline(started_at)?;
    tokio::time::timeout(
        remaining,
        builder
            .build_query_as::<AnalyticsFact>()
            .fetch_all(&mut **transaction),
    )
    .await
    .map_err(|_| read_deadline_exceeded())?
    .map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to read PostgreSQL analytics fact page: {error}"
        ))
    })
}

fn append_fact_filters(
    builder: &mut QueryBuilder<'_, sqlx::Postgres>,
    query: AnalyticsFactReadQuery,
    after_key: Option<&AnalyticsFactKey>,
    high_watermark: Option<&AnalyticsFactKey>,
) {
    match query.scope {
        AnalyticsFactScope::All => {}
        AnalyticsFactScope::ProxyOnly => {
            builder.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        AnalyticsFactScope::Account(account_id) => {
            builder
                .push(" AND upstream_account_id = ")
                .push_bind(account_id);
        }
        AnalyticsFactScope::AccountProxyOnly(account_id) => {
            builder.push(" AND source = ").push_bind(SOURCE_PROXY);
            builder
                .push(" AND upstream_account_id = ")
                .push_bind(account_id);
        }
    }
    if let Some(start_at) = query.start_at {
        builder
            .push(" AND occurred_at_epoch_ms >= ")
            .push_bind(query_epoch_millis(start_at));
    }
    if let Some(end_at) = query.end_at {
        builder
            .push(" AND occurred_at_epoch_ms < ")
            .push_bind(query_epoch_millis(end_at));
    }
    if let Some(source_snapshot_id) = query.source_snapshot_id {
        builder
            .push(" AND source_row_id <= ")
            .push_bind(source_snapshot_id);
    }
    if let Some(after_key) = after_key {
        push_key_predicate(builder, ">", after_key);
    }
    if let Some(high_watermark) = high_watermark {
        push_key_predicate(builder, "<=", high_watermark);
    }
}

// Fact timestamps are stored at integer-millisecond precision. Ceil both bounds so a
// sub-millisecond start excludes its containing millisecond, while a half-open sub-millisecond
// end still includes that containing millisecond and excludes the next one.
fn query_epoch_millis(value: DateTime<Utc>) -> i64 {
    value.timestamp_millis().saturating_add(i64::from(
        !value.timestamp_subsec_nanos().is_multiple_of(1_000_000),
    ))
}

fn push_key_predicate(
    builder: &mut QueryBuilder<'_, sqlx::Postgres>,
    operator: &str,
    key: &AnalyticsFactKey,
) {
    // Keep the DATE column native so the projection cursor indexes can serve this tuple range;
    // SQLx binds the cursor date as text and PostgreSQL performs the explicit cast at the bind.
    builder.push(" AND (stats_date, occurred_at_epoch_ms, source, source_row_id) ");
    builder.push(operator);
    builder.push(" (");
    builder
        .push_bind(key.stats_date.clone())
        .push("::date, ")
        .push_bind(key.occurred_at_epoch_ms)
        .push(", ")
        .push_bind(key.source.clone())
        .push(", ")
        .push_bind(key.source_row_id)
        .push(")");
}

fn validate_fact_page(
    after_key: Option<&AnalyticsFactKey>,
    high_watermark: Option<&AnalyticsFactKey>,
    page: &[AnalyticsFact],
) -> AnalyticsReadResult<()> {
    if page.len() > ANALYTICS_READ_PAGE_MAX_ROWS {
        return Err(invalid_budget("analytics fact page exceeds the row budget"));
    }
    let mut previous = after_key.cloned();
    for fact in page {
        validate_fact_for_projection(fact)?;
        let key = AnalyticsFactKey::from_fact(fact)?;
        if previous.as_ref().is_some_and(|previous| key <= *previous) {
            return Err(invalid_cursor(
                "analytics fact page is not strictly ordered",
            ));
        }
        if high_watermark.is_some_and(|high_watermark| key > *high_watermark) {
            return Err(invalid_cursor(
                "analytics fact page crosses its high watermark",
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

fn encoded_fact_size(fact: &AnalyticsFact) -> AnalyticsReadResult<usize> {
    serde_json::to_vec(fact)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to encode PostgreSQL analytics fact for budget validation: {error}"
            ))
        })
}

fn ensure_read_deadline(started_at: Instant) -> AnalyticsReadResult<()> {
    if started_at.elapsed() >= ANALYTICS_READ_DEADLINE {
        return Err(read_deadline_exceeded());
    }
    Ok(())
}

fn remaining_read_deadline(started_at: Instant) -> AnalyticsReadResult<Duration> {
    ANALYTICS_READ_DEADLINE
        .checked_sub(started_at.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(read_deadline_exceeded)
}

fn read_deadline_exceeded() -> AnalyticsReadError {
    AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
        AnalyticsBackend::Postgres,
        "analytics read deadline exceeded",
    ))
}

pub(crate) fn validate_analytics_replica_freshness(
    state: &AnalyticsReplicaState,
) -> AnalyticsReadResult<()> {
    let updated_at = crate::stats::parse_to_utc_datetime(&state.updated_at).ok_or_else(|| {
        AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
            AnalyticsBackend::Postgres,
            "analytics replica freshness timestamp is invalid",
        ))
    })?;
    let now = Utc::now();
    let stale_after = chrono::Duration::from_std(ANALYTICS_READ_MAX_STALE)
        .expect("analytics freshness budget fits chrono duration");
    if updated_at.signed_duration_since(now) > stale_after {
        return Err(AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
            AnalyticsBackend::Postgres,
            "analytics replica freshness timestamp is too far in the future",
        )));
    }
    let age = now.signed_duration_since(updated_at);
    if age > stale_after {
        return Err(AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
            AnalyticsBackend::Postgres,
            "analytics replica freshness exceeded 15 seconds",
        )));
    }
    Ok(())
}

fn invalid_budget(detail: impl Into<String>) -> AnalyticsReadError {
    AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
        AnalyticsBackend::Postgres,
        detail,
    ))
}

fn invalid_cursor(detail: impl Into<String>) -> AnalyticsReadError {
    AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
        AnalyticsBackend::Postgres,
        detail,
    ))
}

fn invalid_fact(detail: impl Into<String>) -> AnalyticsReadError {
    AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
        AnalyticsBackend::Postgres,
        detail,
    ))
}

fn ensure_tail_budget_complete(
    row_count: usize,
    last_key: Option<&AnalyticsFactKey>,
    high_watermark: Option<&AnalyticsFactKey>,
) -> AnalyticsReadResult<()> {
    if row_count >= ANALYTICS_READ_MAX_TAIL_ROWS && last_key != high_watermark {
        return Err(invalid_budget(
            "analytics fact tail reached its row budget before the high watermark",
        ));
    }
    Ok(())
}

pub(crate) fn validate_fact_for_projection(
    fact: &AnalyticsFact,
) -> AnalyticsReadResult<crate::api::FailureClassification> {
    if fact.fact_version != 1 {
        return Err(invalid_fact("analytics fact version is unsupported"));
    }
    let occurred_at = crate::stats::parse_to_utc_datetime(&fact.occurred_at)
        .ok_or_else(|| invalid_fact("analytics fact occurred-at timestamp is invalid"))?;
    if occurred_at.timestamp_millis() != fact.occurred_at_epoch_ms {
        return Err(invalid_fact(
            "analytics fact occurred-at timestamp disagrees with its epoch value",
        ));
    }
    if occurred_at
        .with_timezone(&Shanghai)
        .date_naive()
        .to_string()
        != fact.stats_date
    {
        return Err(invalid_fact(
            "analytics fact stats date disagrees with its occurred-at timestamp",
        ));
    }
    let status = fact
        .status
        .as_deref()
        .map(str::trim)
        .filter(|status| !status.is_empty());
    if fact.status.is_some() && status.is_none() {
        return Err(invalid_fact(
            "analytics fact status classification is empty",
        ));
    }
    if fact
        .failure_class
        .as_deref()
        .and_then(crate::api::FailureClass::from_db_str)
        .is_none()
        && fact.failure_class.is_some()
    {
        return Err(invalid_fact(
            "analytics fact failure classification is invalid",
        ));
    }
    let classification = crate::api::resolve_failure_classification(
        status,
        None,
        fact.failure_kind.as_deref(),
        fact.failure_class.as_deref(),
        fact.is_actionable.map(i64::from),
    );
    let failure_class = classification.failure_class;
    let legacy_success_status = status.is_some_and(|status| {
        matches!(
            status.to_ascii_lowercase().as_str(),
            "http_200" | "warning_success"
        )
    });
    let has_explicit_failure_kind = fact.failure_kind.as_deref().is_some_and(|failure_kind| {
        let failure_kind = failure_kind.trim();
        !failure_kind.is_empty() && !failure_kind.eq_ignore_ascii_case("none")
    });
    let has_explicit_failure_class = fact
        .failure_class
        .as_deref()
        .is_some_and(|failure_class| !failure_class.trim().eq_ignore_ascii_case("none"));
    if legacy_success_status
        && !fact.success_like
        && !has_explicit_failure_kind
        && !has_explicit_failure_class
    {
        return Err(invalid_fact(
            "analytics fact legacy success status lacks persisted failure classification",
        ));
    }
    let in_flight = status.is_some_and(|status| {
        status.eq_ignore_ascii_case("running") || status.eq_ignore_ascii_case("pending")
    });
    if status.is_some() && fact.terminal == in_flight {
        return Err(invalid_fact(
            "analytics fact terminal classification is inconsistent",
        ));
    }
    let success_like_status = status.is_some_and(|status| {
        matches!(
            status.to_ascii_lowercase().as_str(),
            "success" | "succeeded" | "completed" | "ok" | "http_200" | "warning_success"
        )
    });
    let persisted_success_like = fact.success_like
        && failure_class == crate::api::FailureClass::None
        && (success_like_status || status.is_none());
    if fact.success_like != persisted_success_like {
        return Err(invalid_fact(
            "analytics fact success classification is inconsistent",
        ));
    }
    let success_like = analytics_fact_success_like(fact, failure_class);
    if fact.terminal && !success_like && failure_class == crate::api::FailureClass::None {
        return Err(invalid_fact(
            "terminal analytics fact has no resolved success or failure classification",
        ));
    }
    if !fact.terminal && failure_class != crate::api::FailureClass::None {
        return Err(invalid_fact(
            "in-flight analytics fact has a terminal failure classification",
        ));
    }
    validate_projection_numeric_fields(
        fact.input_tokens,
        fact.output_tokens,
        fact.cache_input_tokens,
        fact.reasoning_tokens,
        fact.total_tokens,
        fact.cost,
        &[
            ("cost_input", fact.cost_input),
            ("cost_cache_write", fact.cost_cache_write),
            ("cost_cache_read", fact.cost_cache_read),
            ("cost_output", fact.cost_output),
            ("cost_reasoning", fact.cost_reasoning),
            ("t_total_ms", fact.t_total_ms),
            ("t_req_read_ms", fact.t_req_read_ms),
            ("t_req_parse_ms", fact.t_req_parse_ms),
            ("t_upstream_connect_ms", fact.t_upstream_connect_ms),
            ("t_upstream_ttfb_ms", fact.t_upstream_ttfb_ms),
            ("t_upstream_stream_ms", fact.t_upstream_stream_ms),
            ("first_token_ms", fact.first_token_ms),
            ("t_resp_parse_ms", fact.t_resp_parse_ms),
        ],
    )?;
    Ok(classification)
}

pub(crate) fn validate_account_window_usage_row(
    row: &AnalyticsAccountUsageRow,
) -> AnalyticsReadResult<()> {
    validate_projection_numeric_fields(
        row.input_tokens,
        row.output_tokens,
        row.cache_input_tokens,
        None,
        row.total_tokens,
        row.cost,
        &[],
    )
}

fn validate_projection_numeric_fields(
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    scalar_values: &[(&str, Option<f64>)],
) -> AnalyticsReadResult<()> {
    for (name, value) in [
        ("input_tokens", input_tokens),
        ("output_tokens", output_tokens),
        ("cache_input_tokens", cache_input_tokens),
        ("reasoning_tokens", reasoning_tokens),
        ("total_tokens", total_tokens),
    ] {
        if value.is_some_and(|value| value < 0) {
            return Err(invalid_fact(format!("analytics fact {name} is invalid")));
        }
    }
    if let (Some(input_tokens), Some(cache_input_tokens)) = (input_tokens, cache_input_tokens)
        && cache_input_tokens > input_tokens
    {
        return Err(invalid_fact(
            "analytics fact cache input tokens exceed input tokens",
        ));
    }
    if let (Some(total_tokens), Some(input_tokens), Some(output_tokens)) =
        (total_tokens, input_tokens, output_tokens)
        && input_tokens.checked_add(output_tokens) != Some(total_tokens)
    {
        return Err(invalid_fact(
            "analytics fact total tokens disagree with input and output tokens",
        ));
    }
    for (name, value) in std::iter::once(("cost", cost)).chain(scalar_values.iter().copied()) {
        if let Some(value) = value
            && (!value.is_finite() || value < 0.0)
        {
            return Err(invalid_fact(format!("analytics fact {name} is invalid")));
        }
    }
    Ok(())
}

pub(crate) fn analytics_fact_to_invocation_record(
    fact: AnalyticsFact,
) -> crate::api::InvocationAggregateRecord {
    let live_phase = analytics_fact_live_phase(&fact);
    let status = fact.status.map(|status| {
        if fact.success_like
            && matches!(
                status.trim().to_ascii_lowercase().as_str(),
                "succeeded" | "ok"
            )
        {
            "success".to_string()
        } else {
            status
        }
    });
    crate::api::InvocationAggregateRecord {
        id: fact.source_row_id,
        invoke_id: fact.invoke_id,
        occurred_at: fact.occurred_at,
        status,
        total_tokens: fact.total_tokens,
        input_tokens: fact.input_tokens,
        output_tokens: fact.output_tokens,
        cache_input_tokens: fact.cache_input_tokens,
        reasoning_tokens: fact.reasoning_tokens,
        cost: fact.cost,
        error_message: None,
        failure_kind: fact.failure_kind,
        failure_class: fact.failure_class,
        is_actionable: fact.is_actionable.map(i64::from),
        live_phase,
        t_total_ms: fact.t_total_ms,
        t_req_read_ms: fact.t_req_read_ms,
        t_req_parse_ms: fact.t_req_parse_ms,
        t_upstream_connect_ms: fact.t_upstream_connect_ms,
        t_upstream_ttfb_ms: fact.t_upstream_ttfb_ms,
        first_token_ms: fact.first_token_ms,
        t_upstream_stream_ms: fact.t_upstream_stream_ms,
        t_resp_parse_ms: fact.t_resp_parse_ms,
        t_persist_ms: None,
    }
}

fn analytics_fact_success_like(
    fact: &AnalyticsFact,
    failure_class: crate::api::FailureClass,
) -> bool {
    (fact.success_like
        || crate::api::prompt_invocation_status_is_success_like(fact.status.as_deref(), None))
        && failure_class == crate::api::FailureClass::None
}

fn analytics_fact_live_phase(fact: &AnalyticsFact) -> Option<String> {
    let status = fact.status.as_deref()?.trim();
    if status.eq_ignore_ascii_case("pending") {
        return Some(crate::api::INVOCATION_LIVE_PHASE_QUEUED.to_string());
    }
    if !status.eq_ignore_ascii_case("running") {
        return None;
    }
    if fact
        .first_token_ms
        .is_some_and(|value| value.is_finite() && value >= 0.0)
    {
        return Some(crate::api::INVOCATION_LIVE_PHASE_RESPONDING.to_string());
    }
    if fact.upstream_account_id.is_some()
        || [
            fact.t_upstream_connect_ms,
            fact.t_req_read_ms,
            fact.t_req_parse_ms,
        ]
        .into_iter()
        .flatten()
        .any(|value| value.is_finite() && value > 0.0)
    {
        return Some(crate::api::INVOCATION_LIVE_PHASE_REQUESTING.to_string());
    }
    Some(crate::api::INVOCATION_LIVE_PHASE_QUEUED.to_string())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AnalyticsSummaryAggregate {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
    pub(crate) non_success_cost: f64,
    pub(crate) non_success_tokens: i64,
    usage: AnalyticsUsageAggregate,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct AnalyticsUsageAggregate {
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    costs: UsageCostBreakdownResponse,
    has_cost: bool,
    models: BTreeMap<(String, Option<String>), AnalyticsUsageModelAggregate>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct AnalyticsUsageModelAggregate {
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    costs: UsageCostBreakdownResponse,
    has_cost: bool,
}

impl AnalyticsSummaryAggregate {
    pub(crate) fn add_fact(&mut self, fact: &AnalyticsFact) -> AnalyticsReadResult<()> {
        let mut candidate = self.clone();
        candidate.add_fact_inner(fact)?;
        *self = candidate;
        Ok(())
    }

    fn add_fact_inner(&mut self, fact: &AnalyticsFact) -> AnalyticsReadResult<()> {
        let classification = validate_fact_for_projection(fact)?;
        let success_like = analytics_fact_success_like(fact, classification.failure_class);
        let non_success =
            fact.terminal && classification.failure_class != crate::api::FailureClass::None;
        self.total_count = checked_add(self.total_count, 1, "total count")?;
        if success_like {
            self.success_count = checked_add(self.success_count, 1, "success count")?;
        } else if non_success {
            self.failure_count = checked_add(self.failure_count, 1, "failure count")?;
        }
        self.total_tokens = checked_add(
            self.total_tokens,
            fact.total_tokens.unwrap_or_default(),
            "total tokens",
        )?;
        self.total_cost =
            checked_float_add(self.total_cost, fact.cost.unwrap_or_default(), "total cost")?;
        if non_success {
            self.non_success_cost = checked_float_add(
                self.non_success_cost,
                fact.cost.unwrap_or_default(),
                "non-success cost",
            )?;
            self.non_success_tokens = checked_add(
                self.non_success_tokens,
                fact.total_tokens.unwrap_or_default(),
                "non-success tokens",
            )?;
        }
        self.usage.add_fact(fact)?;
        Ok(())
    }

    pub(crate) fn merge(&mut self, other: &Self) -> AnalyticsReadResult<()> {
        let mut candidate = self.clone();
        candidate.merge_inner(other)?;
        *self = candidate;
        Ok(())
    }

    fn merge_inner(&mut self, other: &Self) -> AnalyticsReadResult<()> {
        self.total_count = checked_add(self.total_count, other.total_count, "total count")?;
        self.success_count = checked_add(self.success_count, other.success_count, "success count")?;
        self.failure_count = checked_add(self.failure_count, other.failure_count, "failure count")?;
        self.total_tokens = checked_add(self.total_tokens, other.total_tokens, "total tokens")?;
        self.non_success_tokens = checked_add(
            self.non_success_tokens,
            other.non_success_tokens,
            "non-success tokens",
        )?;
        self.total_cost = checked_float_add(self.total_cost, other.total_cost, "total cost")?;
        self.non_success_cost = checked_float_add(
            self.non_success_cost,
            other.non_success_cost,
            "non-success cost",
        )?;
        self.usage.merge(&other.usage)
    }

    pub(crate) fn into_response(
        self,
        maintenance: Option<StatsMaintenanceResponse>,
        include_non_success_tokens: bool,
    ) -> StatsResponse {
        StatsResponse {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
            usage_breakdown: self.usage.into_response(),
            in_progress_conversation_count: None,
            in_progress_retry_conversation_count: None,
            in_progress_avg_wait_ms: None,
            in_progress_phase_counts: None,
            non_success_cost: Some(self.non_success_cost),
            non_success_tokens: include_non_success_tokens.then_some(self.non_success_tokens),
            maintenance,
            data_quality: None,
        }
    }
}

impl AnalyticsUsageAggregate {
    fn add_fact(&mut self, fact: &AnalyticsFact) -> AnalyticsReadResult<()> {
        let input_tokens = fact.input_tokens.unwrap_or_default();
        let cache_read_tokens = fact.cache_input_tokens.unwrap_or_default();
        let cache_write_tokens = input_tokens
            .checked_sub(cache_read_tokens)
            .ok_or_else(|| invalid_fact("analytics usage input token arithmetic underflow"))?;
        self.cache_write_tokens = checked_add(
            self.cache_write_tokens,
            cache_write_tokens,
            "cache write tokens",
        )?;
        self.cache_read_tokens = checked_add(
            self.cache_read_tokens,
            cache_read_tokens,
            "cache read tokens",
        )?;
        self.output_tokens = checked_add(
            self.output_tokens,
            fact.output_tokens.unwrap_or_default(),
            "output tokens",
        )?;
        add_costs(
            &mut self.costs,
            &mut self.has_cost,
            fact.cost,
            [
                fact.cost_input,
                fact.cost_cache_write,
                fact.cost_cache_read,
                fact.cost_output,
                fact.cost_reasoning,
            ],
        )?;
        let model = fact
            .response_model
            .as_deref()
            .or(fact.model.as_deref())
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or("unknown")
            .to_string();
        let reasoning_effort = fact
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|effort| !effort.is_empty())
            .map(str::to_string);
        let model_entry = self.models.entry((model, reasoning_effort)).or_default();
        model_entry.cache_write_tokens = checked_add(
            model_entry.cache_write_tokens,
            cache_write_tokens,
            "model cache write tokens",
        )?;
        model_entry.cache_read_tokens = checked_add(
            model_entry.cache_read_tokens,
            cache_read_tokens,
            "model cache read tokens",
        )?;
        model_entry.output_tokens = checked_add(
            model_entry.output_tokens,
            fact.output_tokens.unwrap_or_default(),
            "model output tokens",
        )?;
        add_costs(
            &mut model_entry.costs,
            &mut model_entry.has_cost,
            fact.cost,
            [
                fact.cost_input,
                fact.cost_cache_write,
                fact.cost_cache_read,
                fact.cost_output,
                fact.cost_reasoning,
            ],
        )
    }

    fn merge(&mut self, other: &Self) -> AnalyticsReadResult<()> {
        self.cache_write_tokens = checked_add(
            self.cache_write_tokens,
            other.cache_write_tokens,
            "cache write tokens",
        )?;
        self.cache_read_tokens = checked_add(
            self.cache_read_tokens,
            other.cache_read_tokens,
            "cache read tokens",
        )?;
        self.output_tokens = checked_add(self.output_tokens, other.output_tokens, "output tokens")?;
        if other.has_cost {
            self.has_cost = true;
            merge_costs(&mut self.costs, &other.costs)?;
        }
        for (key, other_entry) in &other.models {
            let entry = self.models.entry(key.clone()).or_default();
            entry.cache_write_tokens = checked_add(
                entry.cache_write_tokens,
                other_entry.cache_write_tokens,
                "model cache write tokens",
            )?;
            entry.cache_read_tokens = checked_add(
                entry.cache_read_tokens,
                other_entry.cache_read_tokens,
                "model cache read tokens",
            )?;
            entry.output_tokens = checked_add(
                entry.output_tokens,
                other_entry.output_tokens,
                "model output tokens",
            )?;
            if other_entry.has_cost {
                entry.has_cost = true;
                merge_costs(&mut entry.costs, &other_entry.costs)?;
            }
        }
        Ok(())
    }

    fn into_response(self) -> Option<UsageBreakdownResponse> {
        let mut models = self
            .models
            .into_iter()
            .filter_map(|((model, reasoning_effort), entry)| {
                let has_usage = entry.cache_write_tokens > 0
                    || entry.cache_read_tokens > 0
                    || entry.output_tokens > 0
                    || entry.has_cost;
                has_usage.then_some(UsageBreakdownModelResponse {
                    model,
                    reasoning_effort,
                    cache_write_tokens: entry.cache_write_tokens,
                    cache_read_tokens: entry.cache_read_tokens,
                    output_tokens: entry.output_tokens,
                    costs: entry.has_cost.then_some(entry.costs),
                })
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| {
            left.model
                .cmp(&right.model)
                .then_with(|| left.reasoning_effort.cmp(&right.reasoning_effort))
        });
        let has_usage = self.cache_write_tokens > 0
            || self.cache_read_tokens > 0
            || self.output_tokens > 0
            || self.has_cost
            || !models.is_empty();
        has_usage.then_some(UsageBreakdownResponse {
            cache_write_tokens: self.cache_write_tokens,
            cache_read_tokens: self.cache_read_tokens,
            output_tokens: self.output_tokens,
            costs: self.has_cost.then_some(self.costs),
            models,
        })
    }
}

fn add_costs(
    target: &mut UsageCostBreakdownResponse,
    has_cost: &mut bool,
    total_cost: Option<f64>,
    components: [Option<f64>; 5],
) -> AnalyticsReadResult<()> {
    let Some(total_cost) = total_cost else {
        return Ok(());
    };
    *has_cost = true;
    let values = if components.iter().all(Option::is_some) {
        components.map(Option::unwrap_or_default)
    } else {
        [0.0, 0.0, 0.0, 0.0, total_cost]
    };
    target.input = checked_float_add(target.input, values[0], "input cost")?;
    target.cache_write = checked_float_add(target.cache_write, values[1], "cache write cost")?;
    target.cache_read = checked_float_add(target.cache_read, values[2], "cache read cost")?;
    target.output = checked_float_add(target.output, values[3], "output cost")?;
    if components.iter().all(Option::is_some) {
        target.reasoning = checked_float_add(target.reasoning, values[4], "reasoning cost")?;
    } else {
        target.unknown = checked_float_add(target.unknown, values[4], "unknown cost")?;
    }
    Ok(())
}

fn merge_costs(
    target: &mut UsageCostBreakdownResponse,
    other: &UsageCostBreakdownResponse,
) -> AnalyticsReadResult<()> {
    target.input = checked_float_add(target.input, other.input, "input cost")?;
    target.cache_write =
        checked_float_add(target.cache_write, other.cache_write, "cache write cost")?;
    target.cache_read = checked_float_add(target.cache_read, other.cache_read, "cache read cost")?;
    target.output = checked_float_add(target.output, other.output, "output cost")?;
    target.reasoning = checked_float_add(target.reasoning, other.reasoning, "reasoning cost")?;
    target.unknown = checked_float_add(target.unknown, other.unknown, "unknown cost")?;
    Ok(())
}

fn checked_add(target: i64, value: i64, field: &str) -> AnalyticsReadResult<i64> {
    target
        .checked_add(value)
        .ok_or_else(|| invalid_fact(format!("analytics {field} overflowed")))
}

fn checked_float_add(target: f64, value: f64, field: &str) -> AnalyticsReadResult<f64> {
    let result = target + value;
    if result.is_finite() {
        Ok(result)
    } else {
        Err(invalid_fact(format!("analytics {field} overflowed")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_START_EPOCH_MS: i64 = 1_793_318_400_000;

    fn fixture_fact(
        source_row_id: i64,
        occurred_at_epoch_ms: i64,
        status: &str,
        failure_class: &str,
        total_tokens: i64,
        cost: f64,
    ) -> AnalyticsFact {
        let normalized_status = status.trim().to_ascii_lowercase();
        AnalyticsFact {
            fact_version: 1,
            source: "codex_invocations".to_string(),
            source_row_id,
            invoke_id: format!("invoke-{source_row_id}"),
            occurred_at: format!("2026-10-30T00:00:{:02}.000Z", occurred_at_epoch_ms / 1_000),
            occurred_at_epoch_ms: FIXTURE_START_EPOCH_MS + occurred_at_epoch_ms,
            stats_date: "2026-10-30".to_string(),
            model: Some("fixture-model".to_string()),
            request_model: None,
            response_model: Some("fixture-model".to_string()),
            reasoning_effort: None,
            prompt_cache_key_digest: None,
            upstream_account_id: None,
            upstream_account_name: None,
            status: Some(status.to_string()),
            success_like: matches!(
                normalized_status.as_str(),
                "success" | "succeeded" | "completed" | "ok"
            ),
            terminal: !matches!(normalized_status.as_str(), "running" | "pending"),
            failure_kind: (failure_class != "none").then(|| "fixture".to_string()),
            failure_class: Some(failure_class.to_string()),
            is_actionable: Some(failure_class == "service_failure"),
            input_tokens: Some(total_tokens),
            output_tokens: Some(0),
            cache_input_tokens: Some(0),
            reasoning_tokens: Some(0),
            total_tokens: Some(total_tokens),
            cost: Some(cost),
            cost_input: Some(cost),
            cost_cache_write: Some(0.0),
            cost_cache_read: Some(0.0),
            cost_output: Some(0.0),
            cost_reasoning: Some(0.0),
            t_total_ms: Some(1.0),
            t_req_read_ms: Some(1.0),
            t_req_parse_ms: Some(1.0),
            t_upstream_connect_ms: Some(1.0),
            t_upstream_ttfb_ms: Some(1.0),
            t_upstream_stream_ms: Some(1.0),
            first_token_ms: Some(1.0),
            t_resp_parse_ms: Some(1.0),
        }
    }

    #[test]
    fn summary_fixture_aggregates_same_timestamp_ties_and_live_tail() {
        let historical = vec![
            fixture_fact(101, 0, "completed", "none", 100, 1.25),
            fixture_fact(102, 0, "http_500", "service_failure", 30, 0.50),
        ];
        let tail = vec![
            fixture_fact(103, 0, "http_401", "client_failure", 10, 0.10),
            fixture_fact(104, 1_000, "running", "none", 5, 0.05),
        ];
        let mut prefix = AnalyticsSummaryAggregate::default();
        for fact in &historical {
            prefix.add_fact(fact).expect("valid historical fact");
        }
        let mut live_tail = AnalyticsSummaryAggregate::default();
        for fact in &tail {
            live_tail.add_fact(fact).expect("valid live-tail fact");
        }
        prefix.merge(&live_tail).expect("merge exact tail once");
        assert_eq!(prefix.total_count, 4);
        assert_eq!(prefix.success_count, 1);
        assert_eq!(prefix.failure_count, 2);
        assert_eq!(prefix.total_tokens, 145);
        assert!((prefix.total_cost - 1.90).abs() < f64::EPSILON);
        assert!((prefix.non_success_cost - 0.60).abs() < f64::EPSILON);
    }

    #[test]
    fn keyset_page_validation_rejects_overlap_and_crosses_no_high_watermark() {
        let first = fixture_fact(101, 0, "completed", "none", 1, 0.1);
        let second = fixture_fact(102, 0, "completed", "none", 1, 0.1);
        let first_key = AnalyticsFactKey::from_fact(&first).expect("first key");
        let second_key = AnalyticsFactKey::from_fact(&second).expect("second key");
        validate_fact_page(None, Some(&second_key), std::slice::from_ref(&first))
            .expect("first page");
        let error = validate_fact_page(Some(&first_key), Some(&second_key), &[first])
            .expect_err("overlap must fail closed");
        assert!(matches!(error, AnalyticsReadError::Unavailable(_)));
        validate_fact_page(Some(&first_key), Some(&second_key), &[second])
            .expect("strict continuation");
    }

    #[test]
    fn invalid_classification_and_non_finite_numeric_values_fail_closed() {
        let mut fact = fixture_fact(101, 0, "completed", "none", 1, 0.1);
        fact.failure_class = Some("unknown".to_string());
        assert!(matches!(
            validate_fact_for_projection(&fact),
            Err(AnalyticsReadError::Unavailable(_))
        ));
        let mut fact = fixture_fact(102, 0, "completed", "none", 1, 0.1);
        fact.cost = Some(f64::NAN);
        assert!(matches!(
            validate_fact_for_projection(&fact),
            Err(AnalyticsReadError::Unavailable(_))
        ));
        let mut fact = fixture_fact(103, 0, "completed", "none", 3, 0.1);
        fact.input_tokens = Some(1);
        fact.output_tokens = Some(1);
        assert!(matches!(
            validate_fact_for_projection(&fact),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn summary_aggregate_numeric_failures_do_not_partially_mutate_state() {
        let mut add_target = AnalyticsSummaryAggregate {
            total_count: i64::MAX,
            ..Default::default()
        };
        let add_before = add_target.clone();
        let fact = fixture_fact(101, 0, "completed", "none", 1, 0.1);
        assert!(matches!(
            add_target.add_fact(&fact),
            Err(AnalyticsReadError::Unavailable(_))
        ));
        assert_eq!(add_target, add_before);

        let mut merge_target = AnalyticsSummaryAggregate {
            total_count: i64::MAX,
            ..Default::default()
        };
        let merge_before = merge_target.clone();
        let other = AnalyticsSummaryAggregate {
            total_count: 1,
            ..Default::default()
        };
        assert!(matches!(
            merge_target.merge(&other),
            Err(AnalyticsReadError::Unavailable(_))
        ));
        assert_eq!(merge_target, merge_before);
    }

    #[test]
    fn account_window_usage_numeric_values_fail_closed() {
        let mut row = AnalyticsAccountUsageRow {
            source_row_id: 101,
            occurred_at_epoch_ms: FIXTURE_START_EPOCH_MS,
            upstream_account_id: 7,
            input_tokens: Some(2),
            output_tokens: Some(1),
            cache_input_tokens: Some(1),
            total_tokens: Some(3),
            cost: Some(0.1),
        };
        assert!(validate_account_window_usage_row(&row).is_ok());

        row.cache_input_tokens = Some(3);
        assert!(matches!(
            validate_account_window_usage_row(&row),
            Err(AnalyticsReadError::Unavailable(_))
        ));
        row.cache_input_tokens = Some(1);
        row.total_tokens = Some(4);
        assert!(matches!(
            validate_account_window_usage_row(&row),
            Err(AnalyticsReadError::Unavailable(_))
        ));
        row.total_tokens = Some(3);
        row.cost = Some(f64::NAN);
        assert!(matches!(
            validate_account_window_usage_row(&row),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn nullable_legacy_classification_uses_the_operational_resolver() {
        let mut fact = fixture_fact(101, 0, "http_200", "none", 1, 0.1);
        fact.failure_kind = Some("upstream_stream_error".to_string());
        fact.failure_class = None;
        fact.is_actionable = None;
        fact.success_like = false;

        let classification = validate_fact_for_projection(&fact).expect("legacy fact is valid");
        assert_eq!(
            classification.failure_class,
            crate::api::FailureClass::ServiceFailure
        );
        assert!(classification.is_actionable);

        let mut statusless = fixture_fact(102, 1_000, "completed", "none", 1, 0.1);
        statusless.status = None;
        statusless.success_like = false;
        statusless.failure_class = None;
        statusless.is_actionable = None;
        assert!(matches!(
            validate_fact_for_projection(&statusless),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn ambiguous_legacy_success_statuses_fail_closed_without_classification() {
        for status in ["http_200", "warning_success"] {
            let mut fact = fixture_fact(101, 0, status, "none", 1, 0.1);
            fact.failure_kind = None;
            fact.failure_class = None;
            fact.is_actionable = None;
            fact.success_like = false;
            assert!(matches!(
                validate_fact_for_projection(&fact),
                Err(AnalyticsReadError::Unavailable(_))
            ));
        }
        let mut explicitly_none = fixture_fact(103, 0, "http_200", "none", 1, 0.1);
        explicitly_none.failure_kind = None;
        explicitly_none.failure_class = Some("none".to_string());
        explicitly_none.is_actionable = None;
        explicitly_none.success_like = false;
        assert!(matches!(
            validate_fact_for_projection(&explicitly_none),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn unsupported_fact_versions_fail_closed() {
        let mut fact = fixture_fact(101, 0, "completed", "none", 1, 0.1);
        fact.fact_version = 2;
        assert!(matches!(
            validate_fact_for_projection(&fact),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn legacy_success_field_and_live_phase_inference_preserve_read_parity() {
        let mut warning = fixture_fact(101, 0, "warning_success", "none", 1, 0.1);
        warning.success_like = true;
        assert!(validate_fact_for_projection(&warning).is_ok());
        let mut http_200 = fixture_fact(102, 1_000, "http_200", "none", 1, 0.1);
        http_200.success_like = true;
        assert!(validate_fact_for_projection(&http_200).is_ok());
        let mut summary = AnalyticsSummaryAggregate::default();
        summary.add_fact(&warning).expect("warning success fact");
        assert_eq!(summary.success_count, 1);

        warning.status = Some("running".to_string());
        warning.terminal = false;
        warning.success_like = false;
        warning.first_token_ms = None;
        warning.t_upstream_connect_ms = None;
        warning.t_req_read_ms = None;
        warning.t_req_parse_ms = None;
        assert_eq!(
            analytics_fact_to_invocation_record(warning.clone())
                .live_phase
                .as_deref(),
            Some(crate::api::INVOCATION_LIVE_PHASE_QUEUED)
        );
        warning.upstream_account_id = Some(7);
        assert_eq!(
            analytics_fact_to_invocation_record(warning.clone())
                .live_phase
                .as_deref(),
            Some(crate::api::INVOCATION_LIVE_PHASE_REQUESTING)
        );
        warning.first_token_ms = Some(0.0);
        assert_eq!(
            analytics_fact_to_invocation_record(warning)
                .live_phase
                .as_deref(),
            Some(crate::api::INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn legacy_failure_kind_overrides_a_none_classification_for_projection_parity() {
        let mut fact = fixture_fact(101, 0, "http_200", "none", 1, 0.1);
        fact.failure_kind = Some("upstream_stream_error".to_string());
        fact.is_actionable = Some(true);

        let classification = validate_fact_for_projection(&fact).expect("valid legacy fact");
        assert_eq!(
            classification.failure_class,
            crate::api::FailureClass::ServiceFailure
        );
        let mut summary = AnalyticsSummaryAggregate::default();
        summary.add_fact(&fact).expect("legacy failure fact");
        assert_eq!(summary.success_count, 0);
        assert_eq!(summary.failure_count, 1);
    }

    #[test]
    fn successful_status_with_legacy_error_classification_preserves_failure_parity() {
        let mut fact = fixture_fact(101, 0, "completed", "service_failure", 1, 0.1);
        fact.success_like = false;
        assert!(!fact.success_like);
        let classification = validate_fact_for_projection(&fact).expect("valid classified fact");
        assert_eq!(
            classification.failure_class,
            crate::api::FailureClass::ServiceFailure
        );
        let mut summary = AnalyticsSummaryAggregate::default();
        summary.add_fact(&fact).expect("classified fact aggregates");
        assert_eq!(summary.success_count, 0);
        assert_eq!(summary.failure_count, 1);
    }

    #[test]
    fn persisted_success_statuses_are_normalized_for_timeseries_aggregation() {
        for status in ["succeeded", "ok"] {
            let fact = fixture_fact(101, 0, status, "none", 1, 0.1);
            assert_eq!(
                analytics_fact_to_invocation_record(fact).status.as_deref(),
                Some("success")
            );
        }
    }

    #[test]
    fn changed_fence_invalidates_a_restartable_cursor() {
        let fence = AnalyticsReadFence {
            replica_id: ANALYTICS_REPLICA_ID.to_string(),
            scope: AnalyticsFactScope::All,
            identity: AnalyticsReadFenceIdentity {
                generation: 4,
                baseline_generation: 4,
                coverage_generation: 4,
                coverage_revision: 7,
                coverage_watermark: 8,
                source_checkpoint: 8,
                source_acknowledged_through: 8,
            },
            high_watermark: None,
        };
        let cursor = fence.cursor_after(None);
        let mut changed = fence.clone();
        changed.identity.coverage_revision += 1;
        assert!(matches!(
            changed.validate_cursor(&cursor),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn source_snapshot_fence_is_non_negative_and_preserved_on_fact_queries() {
        let query = AnalyticsFactReadQuery::all(AnalyticsFactScope::All).expect("all query");
        let fenced = query
            .with_source_snapshot_id(104)
            .expect("valid source snapshot");
        assert_eq!(fenced.source_snapshot_id, Some(104));
        assert!(matches!(
            query.with_source_snapshot_id(-1),
            Err(AnalyticsReadError::InvalidQuery(_))
        ));
    }

    #[test]
    fn fact_filter_sql_contract_keeps_fences_and_native_date_keysets() {
        let start = Utc
            .timestamp_opt(FIXTURE_START_EPOCH_MS / 1_000, 0)
            .single()
            .expect("exact fixture start");
        let end = start + ChronoDuration::milliseconds(2);
        let query =
            AnalyticsFactReadQuery::for_range(AnalyticsFactScope::AccountProxyOnly(42), start, end)
                .expect("bounded account query")
                .with_source_snapshot_id(104)
                .expect("source snapshot fence");
        let after_key = AnalyticsFactKey {
            stats_date: "2026-10-29".to_string(),
            occurred_at_epoch_ms: FIXTURE_START_EPOCH_MS,
            source: SOURCE_PROXY.to_string(),
            source_row_id: 100,
        };
        let high_watermark = AnalyticsFactKey {
            stats_date: "2026-10-31".to_string(),
            occurred_at_epoch_ms: FIXTURE_START_EPOCH_MS + 2,
            source: SOURCE_PROXY.to_string(),
            source_row_id: 104,
        };
        let mut builder = QueryBuilder::<sqlx::Postgres>::new("SELECT 1 WHERE 1 = 1");
        append_fact_filters(&mut builder, query, Some(&after_key), Some(&high_watermark));
        let sql = builder.sql().to_string();
        assert!(sql.contains("source_row_id <="));
        assert!(sql.contains("occurred_at_epoch_ms >="));
        assert!(sql.contains("occurred_at_epoch_ms <"));
        assert!(sql.contains("(stats_date, occurred_at_epoch_ms, source, source_row_id) >"));
        assert!(sql.contains("(stats_date, occurred_at_epoch_ms, source, source_row_id) <="));
        assert!(sql.contains("::date"));
    }

    #[test]
    fn read_budget_constants_preserve_the_ingestion_page_and_summary_limits() {
        assert_eq!(ANALYTICS_READ_PAGE_MAX_ROWS, 256);
        assert_eq!(ANALYTICS_READ_PAGE_MAX_ENCODED_BYTES, 1024 * 1024);
        assert_eq!(ANALYTICS_READ_MAX_TAIL_ROWS, 200_000);
        assert_eq!(ANALYTICS_READ_MAX_TAIL_ENCODED_BYTES, 32 * 1024 * 1024);
        assert_eq!(ANALYTICS_READ_DEADLINE, Duration::from_secs(15));
        assert_eq!(ANALYTICS_READ_MAX_STALE, Duration::from_secs(15));
    }

    #[test]
    fn stale_replica_state_fails_closed_at_the_read_boundary() {
        let state = AnalyticsReplicaState {
            replica_id: ANALYTICS_REPLICA_ID.to_string(),
            status: "caught_up".to_string(),
            fact_date_semantics_version: ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
            generation: 1,
            baseline_generation: 1,
            baseline_change_id: 1,
            coverage_generation: 1,
            coverage_revision: 1,
            coverage_watermark: 1,
            consumer_checkpoint: 1,
            source_acknowledged_through: 1,
            service_eligible: true,
            last_error_code: None,
            updated_at: (Utc::now() - ChronoDuration::seconds(16)).to_rfc3339(),
        };
        assert!(matches!(
            validate_analytics_replica_freshness(&state),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn materially_future_replica_state_fails_closed_at_the_read_boundary() {
        let state = AnalyticsReplicaState {
            replica_id: ANALYTICS_REPLICA_ID.to_string(),
            status: "caught_up".to_string(),
            fact_date_semantics_version: ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
            generation: 1,
            baseline_generation: 1,
            baseline_change_id: 1,
            coverage_generation: 1,
            coverage_revision: 1,
            coverage_watermark: 1,
            consumer_checkpoint: 1,
            source_acknowledged_through: 1,
            service_eligible: true,
            last_error_code: None,
            updated_at: (Utc::now() + ChronoDuration::seconds(16)).to_rfc3339(),
        };
        assert!(matches!(
            validate_analytics_replica_freshness(&state),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn row_budget_never_returns_a_partial_tail() {
        let fact = fixture_fact(101, 0, "completed", "none", 1, 0.1);
        let key = AnalyticsFactKey::from_fact(&fact).expect("fact key");
        assert!(
            ensure_tail_budget_complete(ANALYTICS_READ_MAX_TAIL_ROWS, Some(&key), Some(&key),)
                .is_ok()
        );

        let later = fixture_fact(102, 1_000, "completed", "none", 1, 0.1);
        let later_key = AnalyticsFactKey::from_fact(&later).expect("later fact key");
        assert!(matches!(
            ensure_tail_budget_complete(ANALYTICS_READ_MAX_TAIL_ROWS, Some(&key), Some(&later_key),),
            Err(AnalyticsReadError::Unavailable(_))
        ));
    }

    #[test]
    fn range_bounds_round_up_submillisecond_instants_for_integer_fact_times() {
        let exact = Utc
            .timestamp_opt(FIXTURE_START_EPOCH_MS / 1_000, 0)
            .single()
            .expect("exact fixture instant");
        assert_eq!(query_epoch_millis(exact), FIXTURE_START_EPOCH_MS);
        assert_eq!(
            query_epoch_millis(exact + ChronoDuration::microseconds(1)),
            FIXTURE_START_EPOCH_MS + 1
        );
        assert_eq!(query_epoch_millis(exact), FIXTURE_START_EPOCH_MS);
        assert_eq!(
            query_epoch_millis(exact + ChronoDuration::milliseconds(1)),
            FIXTURE_START_EPOCH_MS + 1
        );
    }
}
