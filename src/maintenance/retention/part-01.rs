pub(crate) async fn prune_system_task_runs(pool: &Pool<Sqlite>, dry_run: bool) -> Result<usize> {
    let now_epoch_ms = system_task_run_retention_now_epoch_ms();
    if !system_task_run_retention_pass_is_due(now_epoch_ms) {
        return Ok(0);
    }

    let success_cutoff = format_utc_iso_millis(Utc::now() - ChronoDuration::days(30));
    let failed_cutoff = format_utc_iso_millis(Utc::now() - ChronoDuration::days(180));
    if dry_run {
        return dry_run_system_task_retention(pool, &success_cutoff, &failed_cutoff).await;
    }
    let candidates =
        select_system_task_retention_candidates(pool, &success_cutoff, &failed_cutoff).await?;
    if candidates.is_empty() {
        return Ok(0);
    }

    let mut pruned = 0usize;
    for candidates in candidates.chunks(SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS) {
        let Some(deleted) = prune_system_task_run_batch(pool, candidates).await? else {
            break;
        };
        pruned += deleted;
        if deleted < candidates.len() {
            break;
        }
    }
    Ok(pruned)
}

async fn dry_run_system_task_retention(
    pool: &Pool<Sqlite>,
    success_cutoff: &str,
    failed_cutoff: &str,
) -> Result<usize> {
    let candidates: i64 = sqlx::query_scalar(
        r#"
        WITH ranked AS (
            SELECT
                task_kind,
                status,
                started_at,
                ROW_NUMBER() OVER (
                    PARTITION BY task_kind, status
                    ORDER BY started_at DESC, id DESC
                ) AS retention_rank
            FROM system_task_runs
            WHERE status IN ('success', 'skipped', 'failed')
              AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at
        )
        SELECT COUNT(*)
        FROM ranked
        WHERE retention_rank > ?1
          AND (
              (status IN ('success', 'skipped')
                AND (started_at < ?2 OR retention_rank > 5000))
              OR (status = 'failed'
                AND (started_at < ?3 OR retention_rank > 10000))
          )
        "#,
    )
    .bind(SYSTEM_TASK_RUN_RETENTION_KEEP_RECENT)
    .bind(success_cutoff)
    .bind(failed_cutoff)
    .fetch_one(pool)
    .await
    .map_err(anyhow::Error::from)
    .or_else(|error| {
        if system_task_run_retention_handle_pressure(&error) {
            Ok(0)
        } else {
            Err(error.context("failed to count system task retention candidates"))
        }
    })?;
    Ok((candidates.max(0) as usize).min(SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS))
}

async fn select_system_task_retention_candidates(
    pool: &Pool<Sqlite>,
    success_cutoff: &str,
    failed_cutoff: &str,
) -> Result<Vec<SystemTaskRunRetentionCandidate>> {
    match sqlx::query_as::<_, SystemTaskRunRetentionCandidate>(
        r#"
        WITH ranked AS (
            SELECT
                id,
                task_kind,
                status,
                started_at,
                ROW_NUMBER() OVER (
                    PARTITION BY task_kind, status
                    ORDER BY started_at DESC, id DESC
                ) AS retention_rank
            FROM system_task_runs
            WHERE status IN ('success', 'skipped', 'failed')
              AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at
        )
        SELECT id
        FROM ranked
        WHERE retention_rank > ?1
          AND (
              (status IN ('success', 'skipped')
                AND (started_at < ?2 OR retention_rank > 5000))
              OR (status = 'failed'
                AND (started_at < ?3 OR retention_rank > 10000))
          )
        ORDER BY started_at ASC, id ASC
        LIMIT ?4
        "#,
    )
    .bind(SYSTEM_TASK_RUN_RETENTION_KEEP_RECENT)
    .bind(success_cutoff)
    .bind(failed_cutoff)
    .bind(SYSTEM_TASK_RUN_RETENTION_MAX_ROWS_PER_PASS as i64)
    .fetch_all(pool)
    .await
    {
        Ok(candidates) => Ok(candidates),
        Err(error) => {
            let error = anyhow::Error::from(error);
            if system_task_run_retention_handle_pressure(&error) {
                Ok(Vec::new())
            } else {
                Err(error).context("failed to select system task retention candidates")
            }
        }
    }
}

async fn prune_system_task_run_batch(
    pool: &Pool<Sqlite>,
    candidates: &[SystemTaskRunRetentionCandidate],
) -> Result<Option<usize>> {
    let Some(admission) = acquire_retention_write_admission("system_task_run_retention").await
    else {
        if system_task_run_retention_admission_requires_pressure_backoff(
            crate::db_pressure::global_db_pressure_gate().background_deny_reason(),
        ) {
            system_task_run_retention_schedule_next(
                system_task_run_retention_now_epoch_ms(),
                SYSTEM_TASK_RUN_RETENTION_PRESSURE_BACKOFF,
            );
        }
        return Ok(None);
    };
    let execute_started = Instant::now();
    let ids = candidates
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let mut delete = QueryBuilder::<Sqlite>::new("DELETE FROM system_task_runs WHERE id IN (");
    let mut separated = delete.separated(", ");
    for id in &ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    let mut transaction = match pool.begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            let error = anyhow::Error::from(error);
            if system_task_run_retention_handle_pressure(&error) {
                return Ok(Some(0));
            }
            return Err(error).context("failed to begin system task retention batch");
        }
    };
    let deleted = match delete.build().execute(&mut *transaction).await {
        Ok(result) => result.rows_affected() as usize,
        Err(error) => {
            let error = anyhow::Error::from(error);
            if system_task_run_retention_handle_pressure(&error) {
                return Ok(Some(0));
            }
            return Err(error).context("failed to delete system task retention batch");
        }
    };
    if let Err(error) = transaction.commit().await {
        let error = anyhow::Error::from(error);
        if system_task_run_retention_handle_pressure(&error) {
            return Ok(Some(0));
        }
        return Err(error).context("failed to commit system task retention batch");
    }
    retention_record_commit!(
        "system_task_run_retention",
        admission.admission_mode(),
        deleted,
        deleted.saturating_mul(128),
        Duration::ZERO,
        admission.lock_wait(),
        execute_started.elapsed(),
        Duration::ZERO,
        admission.p1_waiter_count(),
        usize::from(candidates.len() == SYSTEM_TASK_RUN_RETENTION_TERMINAL_BATCH_ROWS),
    );
    Ok(Some(deleted))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchiveTableSpec {
    pub(crate) dataset: &'static str,
    pub(crate) columns: &'static str,
    pub(crate) create_sql: &'static str,
}

#[derive(Debug)]
pub(crate) struct ArchiveBatchOutcome {
    pub(crate) dataset: &'static str,
    pub(crate) month_key: String,
    pub(crate) day_key: Option<String>,
    pub(crate) part_key: Option<String>,
    pub(crate) file_path: String,
    pub(crate) sha256: String,
    pub(crate) row_count: i64,
    pub(crate) upstream_last_activity: Vec<(i64, String)>,
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
    pub(crate) archive_expires_at: Option<String>,
    pub(crate) summary_source_kind: &'static str,
    pub(crate) layout: &'static str,
    pub(crate) codec: &'static str,
    pub(crate) writer_version: &'static str,
    pub(crate) cleanup_state: &'static str,
    pub(crate) superseded_by: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct InvocationRollupDelta {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationDetailPruneCandidate {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) request_raw_path: Option<String>,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) estimated_write_bytes: i64,
}

#[derive(Debug, FromRow, Clone)]
pub(crate) struct InvocationArchiveCandidate {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) status: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    #[sqlx(default)]
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) payload: Option<String>,
    pub(crate) request_raw_path: Option<String>,
    pub(crate) response_raw_path: Option<String>,
}

fn invocation_archive_candidate_to_hourly_source_record(
    candidate: &InvocationArchiveCandidate,
) -> InvocationHourlySourceRecord {
    InvocationHourlySourceRecord {
        id: candidate.id,
        occurred_at: candidate.occurred_at.clone(),
        source: candidate.source.clone(),
        status: candidate.status.clone(),
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        model: None,
        input_tokens: candidate.input_tokens,
        output_tokens: candidate.output_tokens,
        cache_input_tokens: candidate.cache_input_tokens,
        reasoning_tokens: candidate.reasoning_tokens,
        total_tokens: candidate.total_tokens,
        cost: candidate.cost,
        upstream_account_id: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        payload: candidate.payload.clone(),
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: candidate.first_token_ms,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
    }
}
