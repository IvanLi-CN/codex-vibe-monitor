pub(crate) async fn begin_system_task_run(
    pool: &Pool<Sqlite>,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    let started_at = format_utc_iso_millis(Utc::now());
    let trigger_kind = trigger_kind.into();
    let id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO system_task_runs (
            task_kind,
            trigger_kind,
            status,
            summary,
            started_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        RETURNING id
        "#,
    )
    .bind(task_kind.as_str())
    .bind(&trigger_kind)
    .bind(SystemTaskStatus::Running.as_str())
    .bind(summary)
    .bind(&started_at)
    .fetch_one(pool)
    .await?;

    Ok(SystemTaskRunHandle {
        id,
        task_kind,
        trigger_kind,
        started_at: Instant::now(),
    })
}

/// Record a task start without allowing SQLite's configured busy timeout to outlive shutdown.
/// This uses a short-lived connection so the normal pool timeout remains unchanged for callers.
pub(crate) async fn begin_system_task_run_nonblocking(
    database_url: &str,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    let options = SqliteConnectOptions::from_str(database_url)
        .context("invalid sqlite database url")?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::ZERO);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let started_at = format_utc_iso_millis(Utc::now());
    let trigger_kind = trigger_kind.into();
    let result = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO system_task_runs (
            task_kind,
            trigger_kind,
            status,
            summary,
            started_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        RETURNING id
        "#,
    )
    .bind(task_kind.as_str())
    .bind(&trigger_kind)
    .bind(SystemTaskStatus::Running.as_str())
    .bind(summary)
    .bind(&started_at)
    .fetch_one(&mut connection)
    .await;
    connection.close().await?;
    let id = result?;

    Ok(SystemTaskRunHandle {
        id,
        task_kind,
        trigger_kind,
        started_at: Instant::now(),
    })
}

pub(crate) async fn begin_system_task_run_admitted(
    state: &AppState,
    write_class: crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let _write_permit = coordinator.acquire(write_class).await;
    begin_system_task_run(&state.pool, task_kind, trigger_kind, summary).await
}

pub(crate) async fn try_begin_system_task_run_with_admission(
    state: &AppState,
    write_class: crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<Option<SystemTaskRunHandle>> {
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let Some(_write_permit) = coordinator.try_acquire(write_class) else {
        return Ok(None);
    };
    begin_system_task_run(&state.pool, task_kind, trigger_kind, summary)
        .await
        .map(Some)
}

pub(crate) async fn finish_system_task_run(
    pool: &Pool<Sqlite>,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) -> bool {
    let finished_at = format_utc_iso_millis(Utc::now());
    let duration_ms = handle
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    if let Err(err) = sqlx::query(
        r#"
        UPDATE system_task_runs
        SET status = ?1,
            summary = COALESCE(?2, summary),
            detail = ?3,
            finished_at = ?4,
            duration_ms = ?5
        WHERE id = ?6
        "#,
    )
    .bind(status.as_str())
    .bind(summary)
    .bind(detail)
    .bind(&finished_at)
    .bind(duration_ms)
    .bind(handle.id)
    .execute(pool)
    .await
    {
        warn!(
            task_kind = handle.task_kind.as_str(),
            trigger_kind = %handle.trigger_kind,
            error = %err,
            "failed to finalize system task run"
        );
        false
    } else {
        true
    }
}

pub(crate) async fn finish_system_task_run_batched(
    state: &AppState,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) {
    if !finish_system_task_run_reliably(state, None, handle, status, summary, detail).await {
        warn!(
            task_kind = handle.task_kind.as_str(),
            trigger_kind = %handle.trigger_kind,
            "failed to durably finalize system task run after bounded retries"
        );
    }
}

const SYSTEM_TASK_FINISH_RETRY_ATTEMPTS: usize = 20;
const SYSTEM_TASK_FINISH_RETRY_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) async fn finish_system_task_run_reliably(
    state: &AppState,
    cancel: Option<&CancellationToken>,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) -> bool {
    let recovery_finish = BatchedSystemTaskFinish {
        run_id: handle.id,
        task_kind: handle.task_kind,
        trigger_kind: handle.trigger_kind.clone(),
        status,
        summary: summary.clone(),
        detail: detail.clone(),
        finished_at: format_utc_iso_millis(Utc::now()),
        duration_ms: handle
            .started_at
            .elapsed()
            .as_millis()
            .min(i64::MAX as u128) as i64,
    };
    for attempt in 0..=SYSTEM_TASK_FINISH_RETRY_ATTEMPTS {
        // Journal and enqueue first. A cancellation path must never perform a direct SQL update
        // while holding a P2 permit: SQLite's busy timeout could otherwise block P1/interactive
        // writers behind a lock held by an external connection.
        if try_enqueue_system_task_run_finish(
            state,
            handle,
            status,
            summary.clone(),
            detail.clone(),
        ) {
            return true;
        }

        if attempt < SYSTEM_TASK_FINISH_RETRY_ATTEMPTS {
            if let Some(cancel) = cancel {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(SYSTEM_TASK_FINISH_RETRY_INTERVAL) => {}
                }
            } else {
                tokio::time::sleep(SYSTEM_TASK_FINISH_RETRY_INTERVAL).await;
            }
        }
    }
    let persisted = state.sqlite_batch_writer.quarantine_system_task_finish(
        &recovery_finish,
        "task-history finish exceeded bounded enqueue retries",
    );
    if !persisted {
        warn!(
            task_kind = handle.task_kind.as_str(),
            trigger_kind = %handle.trigger_kind,
            "failed to persist task-history finish recovery record"
        );
    }
    persisted
}

pub(crate) fn try_enqueue_system_task_run_finish(
    state: &AppState,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) -> bool {
    let finished_at = format_utc_iso_millis(Utc::now());
    let duration_ms = handle
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    let finish = BatchedSystemTaskFinish {
        run_id: handle.id,
        task_kind: handle.task_kind,
        trigger_kind: handle.trigger_kind.clone(),
        status,
        summary,
        detail,
        finished_at,
        duration_ms,
    };
    let journaled = state.sqlite_batch_writer.quarantine_system_task_finish(
        &finish,
        "task-history finish admitted for durable recovery",
    );
    if !journaled {
        warn!(
            run_id = handle.id,
            task_kind = handle.task_kind.as_str(),
            durability_mode = "memory_fallback",
            "task-history finish journal unavailable; enqueueing with reduced durability"
        );
    }
    let enqueued = state
        .sqlite_batch_writer
        .enqueue(SqliteBatchWrite::SystemTaskFinish(finish));
    if !enqueued {
        return false;
    }
    true
}

pub(crate) async fn fetch_system_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SystemStatusResponse>, ApiError> {
    Ok(Json(
        load_system_status_cached(state.as_ref())
            .await
            .map_err(ApiError::unavailable)?,
    ))
}

fn append_system_task_run_filters<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    task_kind: Option<&'a str>,
    status: Option<&'a str>,
    started_at_from: Option<&'a str>,
    started_at_to: Option<&'a str>,
    cursor: Option<&'a SystemTaskRunCursor>,
) {
    if let Some(task_kind) = task_kind {
        builder.push(" AND task_kind = ").push_bind(task_kind);
    }
    if let Some(status) = status {
        builder.push(" AND status = ").push_bind(status);
    }
    if started_at_from.is_some() || started_at_to.is_some() {
        builder.push(" AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at");
    }
    if let Some(started_at_from) = started_at_from {
        builder
            .push(" AND started_at >= ")
            .push_bind(started_at_from);
    }
    if let Some(started_at_to) = started_at_to {
        builder.push(" AND started_at <= ").push_bind(started_at_to);
    }
    if let Some(cursor) = cursor {
        builder
            .push(" AND (started_at < ")
            .push_bind(&cursor.started_at)
            .push(" OR (started_at = ")
            .push_bind(&cursor.started_at)
            .push(" AND id < ")
            .push_bind(cursor.id)
            .push("))");
    }
}

pub(crate) async fn list_system_task_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SystemTaskRunsQuery>,
) -> Result<Json<SystemTaskRunsListResponse>, ApiError> {
    let started_at_from =
        parse_system_task_run_bound(query.started_at_from.as_deref(), "startedAtFrom")?;
    let started_at_to = parse_system_task_run_bound(query.started_at_to.as_deref(), "startedAtTo")?;
    let cursor = parse_system_task_run_cursor(query.cursor.as_deref())?;
    if cursor.is_some() && query.page.unwrap_or(1) > 1 {
        return Err(ApiError::bad_request(anyhow!(
            "cursor cannot be combined with page greater than 1"
        )));
    }
    let page_size = query
        .page_size
        .unwrap_or(query.limit.unwrap_or(20))
        .clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let limit = i64::from(page_size);
    let offset = i64::from(page.saturating_sub(1)) * limit;
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, task_kind, trigger_kind, status, summary, detail, started_at, finished_at, duration_ms FROM system_task_runs WHERE 1 = 1",
    );
    append_system_task_run_filters(
        &mut builder,
        query
            .task_kind
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty()),
        query
            .status
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty()),
        started_at_from.as_deref(),
        started_at_to.as_deref(),
        cursor.as_ref(),
    );
    builder
        .push(" ORDER BY started_at DESC, id DESC LIMIT ")
        .push_bind(limit + 1)
        .push(" OFFSET ")
        .push_bind(offset);
    let mut rows = builder
        .build_query_as::<SystemTaskRunRow>()
        .fetch_all(&state.pool)
        .await?;

    let mut count_builder =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) as total FROM system_task_runs WHERE 1 = 1");
    append_system_task_run_filters(
        &mut count_builder,
        query
            .task_kind
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty()),
        query
            .status
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty()),
        started_at_from.as_deref(),
        started_at_to.as_deref(),
        None,
    );
    let total = count_builder
        .build_query_scalar::<i64>()
        .fetch_one(&state.pool)
        .await?;

    let has_next_page = rows.len() > page_size as usize;
    if has_next_page {
        rows.pop();
    }
    let next_cursor = has_next_page
        .then(|| rows.last().map(encode_system_task_run_cursor))
        .flatten()
        .transpose()?;

    Ok(Json(SystemTaskRunsListResponse {
        items: rows.into_iter().map(Into::into).collect(),
        total: total.max(0) as u64,
        page,
        page_size,
        next_cursor,
    }))
}

pub(crate) fn summarize_retention_run_for_system_task(
    summary: &RetentionRunSummary,
) -> (String, String) {
    let brief = format!(
        "compressed={} archived_invocations={} pruned_details={} model_routes_pruned={} task_runs_pruned={} orphan_raw_removed={}",
        summary.raw_files_compressed,
        summary.invocation_rows_archived,
        summary.invocation_details_pruned,
        summary.model_route_rows_pruned,
        summary.system_task_run_rows_pruned,
        summary.orphan_raw_files_removed
    );
    let detail = format!(
        "dry_run={} raw_candidates={} raw_compressed={} raw_bytes_before={} raw_bytes_after={} details_pruned={} invocation_rows_archived={} forward_proxy_attempt_rows_archived={} pool_attempt_rows_archived={} quota_rows_archived={} archive_batches_touched={} archive_batches_deleted={} raw_files_removed={} model_routes_pruned={} task_runs_pruned={} orphan_raw_files_removed={}",
        summary.dry_run,
        summary.raw_files_compression_candidates,
        summary.raw_files_compressed,
        summary.raw_bytes_before,
        summary.raw_bytes_after,
        summary.invocation_details_pruned,
        summary.invocation_rows_archived,
        summary.forward_proxy_attempt_rows_archived,
        summary.pool_upstream_request_attempt_rows_archived,
        summary.quota_snapshot_rows_archived,
        summary.archive_batches_touched,
        summary.archive_batches_deleted,
        summary.raw_files_removed,
        summary.model_route_rows_pruned,
        summary.system_task_run_rows_pruned,
        summary.orphan_raw_files_removed
    );
    (brief, detail)
}
