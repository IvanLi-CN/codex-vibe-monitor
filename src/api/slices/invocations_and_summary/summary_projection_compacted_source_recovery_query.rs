use super::*;

pub(super) fn push_compacted_source_recovery_filter(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
) -> Result<()> {
    let start = windows
        .iter()
        .map(|(_, start, _)| *start)
        .min()
        .ok_or_else(|| anyhow!("compacted source recovery has no scan windows"))?;
    let end = windows
        .iter()
        .map(|(_, _, end)| *end)
        .max()
        .ok_or_else(|| anyhow!("compacted source recovery has no scan windows"))?;
    query
        .push("occurred_at >= ")
        .push_bind(crate::stats::db_occurred_at_lower_bound(start))
        .push(" AND occurred_at < ")
        .push_bind(crate::db_occurred_at_upper_bound(end))
        .push(" AND id <= ")
        .push_bind(checkpoint.target_live_high_watermark_id)
        .push(" ");
    if let Some(cursor_occurred_at) = checkpoint.cursor_occurred_at.as_deref() {
        query
            .push("AND (occurred_at, id) > (")
            .push_bind(cursor_occurred_at.to_string())
            .push(", ")
            .push_bind(checkpoint.cursor_id)
            .push(") ");
    } else if checkpoint.cursor_id > 0 {
        query
            .push("AND id > ")
            .push_bind(checkpoint.cursor_id)
            .push(" ");
    }
    Ok(())
}

pub(super) fn push_compacted_source_recovery_text_length(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    expression: &str,
) {
    query
        .push(" + length(CAST(COALESCE(")
        .push(expression)
        .push(", '') AS BLOB))");
}

pub(super) fn push_compacted_source_recovery_candidate_columns(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
) {
    query
        .push("SELECT id, invoke_id, occurred_at, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" AS upstream_account_id, model, ")
        .push(INVOCATION_RESPONSE_MODEL_SQL)
        .push(" AS response_model, ")
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(
            " AS reasoning_effort, input_tokens, output_tokens, cache_input_tokens, \
                total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, \
                cost_reasoning, status, CASE WHEN length(CAST(COALESCE(error_message, '') AS BLOB)) > ",
        )
        .push_bind(
            i64::try_from(SUMMARY_COMPACTED_SOURCE_RECOVERY_ERROR_MESSAGE_BYTES).unwrap_or(65_536),
        )
        .push(
            " THEN '__summary_recovery_oversized_error__' ELSE error_message END AS error_message, ",
        )
        .push(
            "NULL AS failure_kind, NULL AS failure_class, NULL AS is_actionable, \
             0 AS estimated_bytes, 0 AS error_message_bytes FROM codex_invocations WHERE ",
        );
}

pub(super) fn push_compacted_source_recovery_candidate_budget_columns(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
) {
    query
        .push("SELECT id, occurred_at, ")
        .push(INVOCATION_STATUS_NORMALIZED_SQL)
        .push(" AS status, ")
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, (192");
    for expression in [
        "invoke_id",
        "occurred_at",
        "model",
        INVOCATION_RESPONSE_MODEL_SQL,
        INVOCATION_REASONING_EFFORT_SQL,
        "status",
        "error_message",
        INVOCATION_FAILURE_KIND_SQL,
        INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    ] {
        push_compacted_source_recovery_text_length(query, expression);
    }
    query.push(
        ") AS estimated_bytes, length(CAST(COALESCE(error_message, '') AS BLOB)) \
         AS error_message_bytes FROM codex_invocations WHERE ",
    );
}

pub(super) async fn open_compacted_source_recovery_connection(
    state: &AppState,
    budget: Duration,
    context: &'static str,
) -> Result<Option<(SqliteConnection, Arc<AtomicBool>, Instant)>> {
    if budget.is_zero() {
        return Ok(None);
    }
    let deadline = Instant::now() + budget;
    let options = state
        .pool
        .connect_options()
        .as_ref()
        .clone()
        .busy_timeout(budget);
    let mut connection =
        match tokio::time::timeout(budget, SqliteConnection::connect_with(&options)).await {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => {
                let error = anyhow!(error).context(context);
                if is_sqlite_contention(&error) {
                    return Ok(None);
                }
                return Err(error);
            }
            Err(_) => return Ok(None),
        };
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let mut handle = connection
            .lock_handle()
            .await
            .context("acquire compacted source recovery progress handle")?;
        handle.set_progress_handler(1_000, move || {
            let keep_running = Instant::now() < deadline;
            if !keep_running {
                interrupted.store(true, Ordering::Release);
            }
            keep_running
        });
    }
    if Instant::now() >= deadline {
        close_compacted_source_recovery_connection(connection).await?;
        return Ok(None);
    }
    Ok(Some((connection, interrupted, deadline)))
}

pub(super) async fn load_compacted_source_recovery_candidates(
    state: &AppState,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
    page_size: usize,
    budget: Duration,
) -> Result<Option<CompactedSourceRecoveryCandidatePage>> {
    let started = Instant::now();
    let mut key_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
    push_compacted_source_recovery_key_page_query(&mut key_query, checkpoint, windows, page_size)?;
    let Some((mut connection, interrupted, deadline)) = open_compacted_source_recovery_connection(
        state,
        budget.saturating_sub(started.elapsed()),
        "open compacted source recovery candidate connection",
    )
    .await?
    else {
        return Ok(None);
    };
    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
        return Ok(None);
    }
    let result = tokio::time::timeout(
        remaining,
        key_query
            .build_query_as::<CompactedSourceRecoveryKey>()
            .fetch_all(&mut connection),
    )
    .await;
    let keys = match result {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => {
            let error = anyhow!(error).context("load bounded compacted source recovery candidates");
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error)
            {
                let _ = close_compacted_source_recovery_connection_bounded(connection, deadline)
                    .await?;
                return Ok(None);
            } else {
                let _ = close_compacted_source_recovery_connection_bounded(connection, deadline)
                    .await?;
                return Err(error);
            }
        }
        Err(_) => {
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            return Ok(None);
        }
    };
    if keys.is_empty() {
        if !close_compacted_source_recovery_connection_bounded(connection, deadline).await? {
            return Ok(None);
        }
        return Ok(Some(CompactedSourceRecoveryCandidatePage::default()));
    }

    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
        return Ok(None);
    }
    let begin_result =
        tokio::time::timeout(remaining, sqlx::query("BEGIN").execute(&mut connection)).await;
    match begin_result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let error = anyhow!(error).context("begin compacted source recovery read snapshot");
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            if is_sqlite_contention(&error) {
                return Ok(None);
            }
            return Err(error);
        }
        Err(_) => {
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            return Ok(None);
        }
    }
    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        let _ =
            rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
                .await?;
        return Ok(None);
    }
    let mut budget_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
    push_compacted_source_recovery_candidate_budget_columns(&mut budget_query);
    budget_query.push("id IN (");
    {
        let mut separated = budget_query.separated(", ");
        for key in &keys {
            separated.push_bind(key.id);
        }
        separated.push_unseparated(")");
    }
    budget_query
        .push(" AND id <= ")
        .push_bind(checkpoint.target_live_high_watermark_id);
    let budget_result = tokio::time::timeout(
        remaining,
        budget_query
            .build_query_as::<CompactedSourceRecoveryCandidateBudget>()
            .fetch_all(&mut connection),
    )
    .await;
    let budgets = match budget_result {
        Ok(Ok(budgets)) => budgets,
        Ok(Err(error)) => {
            let error =
                anyhow!(error).context("load bounded compacted source recovery byte estimates");
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error)
            {
                let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                    connection, deadline,
                )
                .await?;
                return Ok(None);
            } else {
                let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                    connection, deadline,
                )
                .await?;
                return Err(error);
            }
        }
        Err(_) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            return Ok(None);
        }
    };
    let budget_candidates = budgets
        .into_iter()
        .filter_map(CompactedSourceRecoveryCandidateBudget::into_selection_candidate)
        .collect::<Vec<_>>();
    let budget_page = CompactedSourceRecoveryCandidatePage {
        keys: keys.clone(),
        candidates: budget_candidates,
    };
    let (selected_budget_candidates, _, _) =
        match select_compacted_source_recovery_candidate_page(budget_page, page_size) {
            Ok(selection) => selection,
            Err(error) => {
                let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                    connection, deadline,
                )
                .await?;
                return Err(error);
            }
        };
    let selected_budget_by_id = selected_budget_candidates
        .iter()
        .map(|candidate| (candidate.id, candidate.clone()))
        .collect::<HashMap<_, _>>();
    let selected_ids = selected_budget_candidates
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    if selected_ids.is_empty() {
        if !rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
            .await?
        {
            return Ok(None);
        }
        return Ok(Some(CompactedSourceRecoveryCandidatePage {
            keys,
            candidates: Vec::new(),
        }));
    }

    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        let _ =
            rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
                .await?;
        return Ok(None);
    }
    let mut candidate_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
    push_compacted_source_recovery_candidate_columns(&mut candidate_query);
    candidate_query.push("id IN (");
    {
        let mut separated = candidate_query.separated(", ");
        for id in selected_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    candidate_query
        .push(" AND id <= ")
        .push_bind(checkpoint.target_live_high_watermark_id);
    let candidate_result = tokio::time::timeout(
        remaining,
        candidate_query
            .build_query_as::<CompactedSourceRecoveryCandidate>()
            .fetch_all(&mut connection),
    )
    .await;
    let closed =
        rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
            .await?;
    if !closed {
        return Ok(None);
    }
    match candidate_result {
        Ok(Ok(candidates)) => {
            let candidates = candidates
                .into_iter()
                .filter_map(|mut candidate| {
                    let budget = selected_budget_by_id.get(&candidate.id)?;
                    candidate.failure_kind = budget.failure_kind.clone();
                    candidate.failure_class = budget.failure_class.clone();
                    candidate.is_actionable = budget.is_actionable;
                    candidate.estimated_bytes = budget.estimated_bytes;
                    candidate.error_message_bytes = budget.error_message_bytes;
                    Some(candidate)
                })
                .collect();
            Ok(Some(CompactedSourceRecoveryCandidatePage {
                keys,
                candidates,
            }))
        }
        Ok(Err(error)) => {
            let error =
                anyhow!(error).context("load bounded compacted source recovery materialized page");
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error)
            {
                Ok(None)
            } else {
                Err(error)
            }
        }
        Err(_) => Ok(None),
    }
}

pub(super) fn push_compacted_source_recovery_key_page_query(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
    page_size: usize,
) -> Result<()> {
    query.push(
        "SELECT id, occurred_at FROM codex_invocations INDEXED BY \
         idx_codex_invocations_occurred_at_id WHERE ",
    );
    push_compacted_source_recovery_filter(query, checkpoint, windows)?;
    query
        .push(" ORDER BY occurred_at ASC, id ASC LIMIT ")
        .push_bind(i64::try_from(page_size.max(1)).unwrap_or(1));
    Ok(())
}

pub(super) fn add_compacted_source_row_contributions(
    contributions: &mut CompactedSourceRecoveryContributions,
    row: &CompactedSourceRecoverySourceRow,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
) -> Result<()> {
    let occurred_at = crate::stats::parse_to_utc_datetime(&row.occurred_at)
        .ok_or_else(|| anyhow!("invalid canonical occurred_at for row {}", row.id))?;
    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    let status = normalized_runtime_text(row.status.as_deref());
    let success = matches!(
        status.as_str(),
        "success" | "completed" | INVOCATION_STATUS_WARNING_SUCCESS
    ) || (status == "http_200"
        && normalized_runtime_text(row.error_message.as_deref()).is_empty());
    let failure = matches!(
        classification.failure_class,
        FailureClass::ServiceFailure | FailureClass::ClientFailure | FailureClass::ClientAbort
    );
    let total_tokens = row.total_tokens.unwrap_or_default();
    let total_cost = row.cost.unwrap_or_default();
    let cache_read_tokens = row.cache_input_tokens.unwrap_or_default().max(0);
    let cache_write_tokens = row
        .input_tokens
        .unwrap_or_default()
        .max(0)
        .saturating_sub(cache_read_tokens);
    let output_tokens = row.output_tokens.unwrap_or_default().max(0);
    let model = row
        .response_model
        .as_deref()
        .or(row.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let reasoning_effort = row
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string();
    let cost_complete = row.cost_input.is_some()
        && row.cost_cache_write.is_some()
        && row.cost_cache_read.is_some()
        && row.cost_output.is_some()
        && row.cost_reasoning.is_some();
    let usage = CompactedSourceRecoveryUsageDelta {
        cache_write_tokens,
        cache_read_tokens,
        output_tokens,
        cost_input: row.cost_input.unwrap_or_default(),
        cost_cache_write: row.cost_cache_write.unwrap_or_default(),
        cost_cache_read: row.cost_cache_read.unwrap_or_default(),
        cost_output: row.cost_output.unwrap_or_default(),
        cost_reasoning: row.cost_reasoning.unwrap_or_default(),
        cost_unknown: if row.cost.is_some() && !cost_complete {
            total_cost
        } else {
            0.0
        },
        has_cost: i64::from(row.cost.is_some()),
    };
    for (window_name, start, end) in windows {
        if occurred_at < *start || occurred_at >= *end {
            continue;
        }
        let aggregate = CompactedSourceRecoveryAggregateDelta {
            total_count: 1,
            success_count: i64::from(success && classification.failure_class == FailureClass::None),
            failure_count: i64::from(failure),
            total_tokens,
            total_cost,
            non_success_cost: if failure { total_cost } else { 0.0 },
            non_success_tokens: if failure { total_tokens.max(0) } else { 0 },
        };
        let account_scopes =
            std::iter::once(0).chain(row.upstream_account_id.filter(|account| *account > 0));
        for account_scope in account_scopes {
            let aggregate_entry = contributions
                .aggregates
                .entry((window_name.clone(), account_scope))
                .or_default();
            aggregate_entry.total_count += aggregate.total_count;
            aggregate_entry.success_count += aggregate.success_count;
            aggregate_entry.failure_count += aggregate.failure_count;
            aggregate_entry.total_tokens += aggregate.total_tokens;
            aggregate_entry.total_cost += aggregate.total_cost;
            aggregate_entry.non_success_cost += aggregate.non_success_cost;
            aggregate_entry.non_success_tokens += aggregate.non_success_tokens;

            let usage_entry = contributions
                .usage
                .entry((
                    window_name.clone(),
                    account_scope,
                    model.clone(),
                    reasoning_effort.clone(),
                ))
                .or_default();
            usage_entry.cache_write_tokens += usage.cache_write_tokens;
            usage_entry.cache_read_tokens += usage.cache_read_tokens;
            usage_entry.output_tokens += usage.output_tokens;
            usage_entry.cost_input += usage.cost_input;
            usage_entry.cost_cache_write += usage.cost_cache_write;
            usage_entry.cost_cache_read += usage.cost_cache_read;
            usage_entry.cost_output += usage.cost_output;
            usage_entry.cost_reasoning += usage.cost_reasoning;
            usage_entry.cost_unknown += usage.cost_unknown;
            usage_entry.has_cost = i64::from(usage_entry.has_cost > 0 || usage.has_cost > 0);

            contributions.records.push((
                window_name.clone(),
                account_scope,
                row.id,
                row.invoke_id.clone(),
                row.occurred_at.clone(),
            ));
            contributions
                .record_scopes
                .insert((window_name.clone(), account_scope));
        }
    }
    Ok(())
}

pub(super) fn select_compacted_source_recovery_candidates(
    page: Vec<CompactedSourceRecoveryCandidate>,
) -> Result<(Vec<CompactedSourceRecoveryCandidate>, bool)> {
    let keys = page
        .iter()
        .map(|candidate| CompactedSourceRecoveryKey {
            id: candidate.id,
            occurred_at: candidate.occurred_at.clone(),
        })
        .collect();
    let (selected, _cursor, completed) = select_compacted_source_recovery_candidate_page(
        CompactedSourceRecoveryCandidatePage {
            keys,
            candidates: page,
        },
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
    )?;
    Ok((selected, completed))
}

pub(super) fn select_compacted_source_recovery_candidate_page(
    page: CompactedSourceRecoveryCandidatePage,
    page_size: usize,
) -> Result<(
    Vec<CompactedSourceRecoveryCandidate>,
    Option<(String, i64)>,
    bool,
)> {
    let fetched_count = page.keys.len();
    let candidates_by_id = page
        .candidates
        .into_iter()
        .map(|candidate| (candidate.id, candidate))
        .collect::<HashMap<_, _>>();
    let mut selected_bytes = 0usize;
    let mut selected_rows = Vec::with_capacity(candidates_by_id.len().min(fetched_count));
    let mut cursor = None;
    let mut stopped_for_materialization_budget = false;
    let mut stopped_for_unresolved_source_row = false;
    for key in page.keys {
        let Some(row) = candidates_by_id.get(&key.id) else {
            // Running and pending source rows are not eligible for aggregation yet. Keep them at
            // the page boundary so a later terminal transition remains visible to this recovery
            // generation instead of being skipped by the tuple cursor.
            stopped_for_unresolved_source_row = true;
            break;
        };
        if selected_rows.len() >= page_size.max(1) {
            stopped_for_materialization_budget = true;
            break;
        }
        let row_bytes = usize::try_from(row.estimated_bytes)
            .context("compacted source recovery byte estimate overflow")?;
        if selected_rows.is_empty() && row_bytes > SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES {
            let error_message_bytes = usize::try_from(row.error_message_bytes)
                .context("compacted source recovery error message byte estimate overflow")?;
            let non_error_bytes = row_bytes.saturating_sub(error_message_bytes);
            let bounded_materialized_bytes = non_error_bytes.saturating_add(
                error_message_bytes.min(SUMMARY_COMPACTED_SOURCE_RECOVERY_ERROR_MESSAGE_BYTES),
            );
            if bounded_materialized_bytes > SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES {
                bail!(
                    "canonical source row {} exceeds the bounded recovery materialization budget",
                    row.id
                );
            }
            // An oversized raw row is admitted only when the bounded error-message projection
            // brings its materialized text back under the page budget. Other oversized fields
            // fail closed before they can be loaded into Rust.
            selected_rows.push(row.clone());
            cursor = Some((key.occurred_at, key.id));
            stopped_for_materialization_budget = true;
            break;
        }
        if !selected_rows.is_empty()
            && selected_bytes.saturating_add(row_bytes)
                > SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES
        {
            stopped_for_materialization_budget = true;
            break;
        }
        selected_bytes = selected_bytes.saturating_add(row_bytes);
        selected_rows.push(row.clone());
        cursor = Some((key.occurred_at, key.id));
    }
    let completed = !stopped_for_materialization_budget
        && !stopped_for_unresolved_source_row
        && fetched_count < page_size.max(1);
    Ok((selected_rows, cursor, completed))
}

pub(super) async fn stage_compacted_source_recovery_page(
    state: &AppState,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    rows: &[CompactedSourceRecoverySourceRow],
    cursor: Option<&(String, i64)>,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
    completed: bool,
    transaction_budget: Duration,
) -> Result<CompactedSourceRecoveryPageOutcome> {
    let transaction_started = Instant::now();
    let deadline = transaction_started + transaction_budget;
    let options = state
        .pool
        .connect_options()
        .as_ref()
        .clone()
        .busy_timeout(transaction_budget);
    let mut connection =
        match tokio::time::timeout(transaction_budget, SqliteConnection::connect_with(&options))
            .await
        {
            Ok(result) => result.context("open compacted source recovery page connection")?,
            Err(_) => {
                return Ok(CompactedSourceRecoveryPageOutcome::Progress {
                    completed: false,
                    deferred: true,
                });
            }
        };
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let mut handle = connection
            .lock_handle()
            .await
            .context("acquire compacted source recovery progress handle")?;
        handle.set_progress_handler(1_000, move || {
            let keep_running = Instant::now() < deadline;
            if !keep_running {
                interrupted.store(true, Ordering::Release);
            }
            keep_running
        });
    }
    let remaining_budget = transaction_budget.saturating_sub(transaction_started.elapsed());
    if remaining_budget.is_zero() {
        let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
        return Ok(CompactedSourceRecoveryPageOutcome::Progress {
            completed: false,
            deferred: true,
        });
    }
    let begin_result = tokio::time::timeout(
        remaining_budget,
        sqlx::query("BEGIN IMMEDIATE").execute(&mut connection),
    )
    .await;
    match begin_result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let budget_expired = interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
            let contention = is_sqlite_contention(&anyhow!(error.to_string()));
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            if budget_expired || contention {
                return Ok(CompactedSourceRecoveryPageOutcome::Progress {
                    completed: false,
                    deferred: true,
                });
            }
            return Err(anyhow!(error).context("begin compacted source recovery page transaction"));
        }
        Err(_) => {
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            return Ok(CompactedSourceRecoveryPageOutcome::Progress {
                completed: false,
                deferred: true,
            });
        }
    }
    let remaining_budget = transaction_budget.saturating_sub(transaction_started.elapsed());
    if remaining_budget.is_zero() {
        let _ =
            rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
                .await?;
        return Ok(CompactedSourceRecoveryPageOutcome::Progress {
            completed: false,
            deferred: true,
        });
    }
    let result = tokio::time::timeout(
        remaining_budget,
        stage_compacted_source_recovery_page_in_transaction(
            state,
            checkpoint,
            rows,
            cursor,
            windows,
            completed,
            transaction_started,
            &mut connection,
        ),
    )
    .await;
    match result {
        Ok(Ok(CompactedSourceRecoveryPageOutcome::Reset)) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            Ok(CompactedSourceRecoveryPageOutcome::Reset)
        }
        Ok(Ok(CompactedSourceRecoveryPageOutcome::Stale)) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            Ok(CompactedSourceRecoveryPageOutcome::Stale)
        }
        Ok(Ok(outcome)) => {
            let remaining_budget = transaction_budget.saturating_sub(transaction_started.elapsed());
            if remaining_budget.is_zero() {
                let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                    connection, deadline,
                )
                .await?;
                return Ok(CompactedSourceRecoveryPageOutcome::Progress {
                    completed: false,
                    deferred: true,
                });
            }
            // Commit is the transaction's atomic boundary. The dedicated connection is closed
            // on every branch, so a timeout cannot return an in-flight rollback to the pool.
            match tokio::time::timeout(
                remaining_budget,
                sqlx::query("COMMIT").execute(&mut connection),
            )
            .await
            {
                Ok(result) => match result {
                    Ok(_) => {
                        if close_compacted_source_recovery_connection_bounded(connection, deadline)
                            .await?
                        {
                            Ok(outcome)
                        } else {
                            Ok(CompactedSourceRecoveryPageOutcome::Progress {
                                completed: false,
                                deferred: true,
                            })
                        }
                    }
                    Err(error) => {
                        let budget_expired =
                            interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
                        let contention = is_sqlite_contention(&anyhow!(error.to_string()));
                        let closed = close_compacted_source_recovery_connection_bounded(
                            connection, deadline,
                        )
                        .await?;
                        if budget_expired || contention || !closed {
                            Ok(CompactedSourceRecoveryPageOutcome::Progress {
                                completed: false,
                                deferred: true,
                            })
                        } else {
                            Err(anyhow!(error).context("commit compacted source recovery page"))
                        }
                    }
                },
                Err(_) => {
                    let _ =
                        close_compacted_source_recovery_connection_bounded(connection, deadline)
                            .await?;
                    Ok(CompactedSourceRecoveryPageOutcome::Progress {
                        completed: false,
                        deferred: true,
                    })
                }
            }
        }
        Ok(Err(error)) => {
            let budget_expired = interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
            let cleaned = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            if budget_expired || !cleaned {
                Ok(CompactedSourceRecoveryPageOutcome::Progress {
                    completed: false,
                    deferred: true,
                })
            } else {
                Err(error)
            }
        }
        Err(_) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            Ok(CompactedSourceRecoveryPageOutcome::Progress {
                completed: false,
                deferred: true,
            })
        }
    }
}

pub(super) async fn close_compacted_source_recovery_connection(
    mut connection: SqliteConnection,
) -> Result<()> {
    let cleanup = connection.lock_handle().await.map(|mut handle| {
        handle.remove_progress_handler();
    });
    let closed = connection.close().await;
    cleanup.context("remove compacted source recovery progress handler")?;
    closed.context("close compacted source recovery connection")?;
    Ok(())
}

pub(super) async fn close_compacted_source_recovery_connection_bounded(
    connection: SqliteConnection,
    deadline: Instant,
) -> Result<bool> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        drop(connection);
        return Ok(false);
    }
    match tokio::time::timeout(
        remaining,
        close_compacted_source_recovery_connection(connection),
    )
    .await
    {
        Ok(result) => {
            result?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

pub(super) async fn rollback_and_close_compacted_source_recovery_connection_bounded(
    mut connection: SqliteConnection,
    deadline: Instant,
) -> Result<bool> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        drop(connection);
        return Ok(false);
    }
    if tokio::time::timeout(remaining, sqlx::query("ROLLBACK").execute(&mut connection))
        .await
        .is_err()
    {
        drop(connection);
        return Ok(false);
    }
    close_compacted_source_recovery_connection_bounded(connection, deadline).await
}

pub(super) async fn rollback_compacted_source_recovery_transaction_bounded(
    connection: &mut SqliteConnection,
    deadline: Instant,
) -> bool {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return false;
    }
    matches!(
        tokio::time::timeout(remaining, sqlx::query("ROLLBACK").execute(&mut *connection),).await,
        Ok(Ok(_))
    )
}

pub(super) async fn stage_compacted_source_recovery_page_in_transaction(
    state: &AppState,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    rows: &[CompactedSourceRecoverySourceRow],
    cursor: Option<&(String, i64)>,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
    completed: bool,
    transaction_started: Instant,
    connection: &mut SqliteConnection,
) -> Result<CompactedSourceRecoveryPageOutcome> {
    let Some(current_checkpoint) = load_compacted_source_recovery_checkpoint_tx(connection).await?
    else {
        return Ok(CompactedSourceRecoveryPageOutcome::Stale);
    };
    if !compacted_source_recovery_checkpoint_progress_matches(checkpoint, &current_checkpoint) {
        return Ok(CompactedSourceRecoveryPageOutcome::Stale);
    }
    let current_fence = load_summary_projection_generation_fence_tx(state, connection).await?;
    if !current_fence.coverage_sources_match(compacted_source_recovery_fence(checkpoint)) {
        return Ok(CompactedSourceRecoveryPageOutcome::Reset);
    }

    let mut contributions = CompactedSourceRecoveryContributions::default();
    for row in rows {
        add_compacted_source_row_contributions(&mut contributions, row, windows)?;
    }

    if !contributions.aggregates.is_empty() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO summary_compacted_source_recovery_aggregate (\
                scope, window_name, upstream_account_id, total_count, success_count, failure_count,\
                total_tokens, total_cost, non_success_cost, non_success_tokens\
            ) ",
        );
        query.push_values(
            contributions.aggregates.iter(),
            |mut values, ((window, account), delta)| {
                values
                    .push_bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
                    .push_bind(window)
                    .push_bind(*account)
                    .push_bind(delta.total_count)
                    .push_bind(delta.success_count)
                    .push_bind(delta.failure_count)
                    .push_bind(delta.total_tokens)
                    .push_bind(delta.total_cost)
                    .push_bind(delta.non_success_cost)
                    .push_bind(delta.non_success_tokens);
            },
        );
        query.push(
            " ON CONFLICT(scope, window_name, upstream_account_id) DO UPDATE SET \
              total_count = total_count + excluded.total_count, \
              success_count = success_count + excluded.success_count, \
              failure_count = failure_count + excluded.failure_count, \
              total_tokens = total_tokens + excluded.total_tokens, \
              total_cost = total_cost + excluded.total_cost, \
              non_success_cost = non_success_cost + excluded.non_success_cost, \
              non_success_tokens = non_success_tokens + excluded.non_success_tokens",
        );
        query
            .build()
            .execute(&mut *connection)
            .await
            .context("stage compacted source recovery aggregates")?;
    }
    if !contributions.usage.is_empty() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO summary_compacted_source_recovery_usage (\
                scope, window_name, upstream_account_id, normalized_model, normalized_reasoning_effort,\
                cache_write_tokens, cache_read_tokens, output_tokens, cost_input, cost_cache_write,\
                cost_cache_read, cost_output, cost_reasoning, cost_unknown, has_cost\
            ) ",
        );
        query.push_values(
            contributions.usage.iter(),
            |mut values, ((window, account, model, reasoning), delta)| {
                values
                    .push_bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
                    .push_bind(window)
                    .push_bind(*account)
                    .push_bind(model)
                    .push_bind(reasoning)
                    .push_bind(delta.cache_write_tokens)
                    .push_bind(delta.cache_read_tokens)
                    .push_bind(delta.output_tokens)
                    .push_bind(delta.cost_input)
                    .push_bind(delta.cost_cache_write)
                    .push_bind(delta.cost_cache_read)
                    .push_bind(delta.cost_output)
                    .push_bind(delta.cost_reasoning)
                    .push_bind(delta.cost_unknown)
                    .push_bind(delta.has_cost);
            },
        );
        query.push(
            " ON CONFLICT(scope, window_name, upstream_account_id, normalized_model, normalized_reasoning_effort) DO UPDATE SET \
              cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens, \
              cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens, \
              output_tokens = output_tokens + excluded.output_tokens, \
              cost_input = cost_input + excluded.cost_input, \
              cost_cache_write = cost_cache_write + excluded.cost_cache_write, \
              cost_cache_read = cost_cache_read + excluded.cost_cache_read, \
              cost_output = cost_output + excluded.cost_output, \
              cost_reasoning = cost_reasoning + excluded.cost_reasoning, \
              cost_unknown = cost_unknown + excluded.cost_unknown, \
              has_cost = MAX(has_cost, excluded.has_cost)",
        );
        query
            .build()
            .execute(&mut *connection)
            .await
            .context("stage compacted source recovery usage")?;
    }
    if !contributions.records.is_empty() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT OR IGNORE INTO summary_compacted_source_recovery_record (\
                scope, window_name, upstream_account_id, row_id, invoke_id, occurred_at\
            ) ",
        );
        query.push_values(
            contributions.records.iter(),
            |mut values, (window, account, row_id, invoke_id, occurred_at)| {
                values
                    .push_bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
                    .push_bind(window)
                    .push_bind(*account)
                    .push_bind(*row_id)
                    .push_bind(invoke_id)
                    .push_bind(occurred_at);
            },
        );
        query
            .build()
            .execute(&mut *connection)
            .await
            .context("stage compacted source recovery record identities")?;
    }
    for (window, account) in &contributions.record_scopes {
        sqlx::query(
            "DELETE FROM summary_compacted_source_recovery_record \
              WHERE scope = ?1 AND window_name = ?2 AND upstream_account_id = ?3 \
                AND row_id NOT IN (\
                    SELECT row_id FROM summary_compacted_source_recovery_record \
                     WHERE scope = ?1 AND window_name = ?2 AND upstream_account_id = ?3 \
                     ORDER BY occurred_at DESC, row_id DESC LIMIT ?4\
                )",
        )
        .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
        .bind(window)
        .bind(*account)
        .bind(checkpoint.list_limit_max.max(1))
        .execute(&mut *connection)
        .await
        .context("prune compacted source recovery record identities")?;
    }

    let next_state = if completed {
        "completed"
    } else if transaction_started.elapsed() >= SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET
    {
        "deferred"
    } else {
        "running"
    };
    sqlx::query(
        "UPDATE summary_compacted_source_recovery_checkpoint SET \
            cursor_occurred_at = COALESCE(?1, cursor_occurred_at), \
            cursor_id = CASE WHEN ?2 > 0 THEN ?2 ELSE cursor_id END, \
            processed_rows = processed_rows + ?3, state = ?4, last_error_code = NULL, \
            last_error_message = NULL, updated_at = datetime('now') WHERE scope = ?5",
    )
    .bind(cursor.map(|(occurred_at, _)| occurred_at.clone()))
    .bind(cursor.map(|(_, id)| *id).unwrap_or_default())
    .bind(i64::try_from(rows.len()).context("compacted source recovery row count overflow")?)
    .bind(next_state)
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .execute(&mut *connection)
    .await
    .context("advance compacted source recovery checkpoint")?;
    Ok(CompactedSourceRecoveryPageOutcome::Progress {
        completed,
        deferred: next_state == "deferred",
    })
}
