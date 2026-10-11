use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct LongTermIntegrityTotals {
    pub(super) calls: i64,
    pub(super) token_total: i64,
    pub(super) cost_total: f64,
}

#[derive(Debug, Clone)]
pub(super) struct LongTermIntegrityOracle {
    pub(super) date: NaiveDate,
    pub(super) daily: LongTermIntegrityTotals,
    pub(super) hourly: HashMap<i64, LongTermIntegrityTotals>,
}

#[derive(Debug, Clone)]
pub(super) struct LongTermIntegrityMismatch {
    pub(super) date: NaiveDate,
    pub(super) expected: LongTermIntegrityTotals,
    pub(super) observed: LongTermIntegrityTotals,
    pub(super) reason: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct LongTermIntegrityHourRow {
    pub(super) bucket_start_epoch: i64,
    pub(super) calls: i64,
    pub(super) token_total: i64,
    pub(super) cost_total: f64,
}

pub(crate) async fn bootstrap_long_term_integrity_source_boundary_for_legacy_rollups(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    // Earlier schemas had no terminal proof or retirement boundary. The existing canonical
    // history may therefore predate every source that this version can verify. Keep it outside
    // the reconstructable window instead of treating source absence as proof of an empty day.
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    sqlx::query(
        r#"
        UPDATE long_term_stats_state
        SET integrity_source_start_date = COALESCE(integrity_source_start_date, ?1),
            integrity_source_pending_start_date = NULL,
            updated_at = datetime('now')
        WHERE id = ?2
        "#,
    )
    .bind(today)
    .bind(LONG_TERM_STATE_ID)
    .execute(pool)
    .await
    .context("failed to bootstrap legacy long-term integrity source boundary")?;
    Ok(())
}

pub(super) fn long_term_day_epoch_bounds(date: NaiveDate) -> Option<(i64, i64)> {
    let start = date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())?
        .timestamp();
    let end = date
        .succ_opt()?
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())?
        .timestamp();
    Some((start, end))
}

pub(super) fn long_term_bucket_date(bucket_start_epoch: i64) -> Option<NaiveDate> {
    Shanghai
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|value| value.date_naive())
}

pub(super) fn long_term_reconstructable_start(
    retention_start: NaiveDate,
    statistics_start_date: Option<&str>,
    integrity_source_start_date: Option<&str>,
) -> NaiveDate {
    let persisted_start = statistics_start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .unwrap_or(retention_start);
    let integrity_source_start = integrity_source_start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .unwrap_or(retention_start);
    retention_start
        .max(persisted_start)
        .max(integrity_source_start)
}

pub(super) fn long_term_source_safe_start_after_effective_date(date: NaiveDate) -> NaiveDate {
    date.succ_opt().unwrap_or(date)
}

pub(super) fn long_term_unreadable_source_start(
    archive_path: &ArchiveBatchPathRow,
    retention_start: NaiveDate,
) -> NaiveDate {
    archive_path
        .coverage_start_at()
        .and_then(long_term_archive_end_date)
        .unwrap_or(retention_start)
}

pub(super) async fn clear_long_term_invocation_replay_markers_for_unavailable_sources(
    pool: &Pool<Sqlite>,
    file_paths: &[String],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let unique_paths = file_paths.iter().collect::<HashSet<_>>();
    for file_path in unique_paths {
        let (mut transaction, permit) = control.begin(pool).await?;
        let cleared = sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(file_path)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        control.commit(transaction, permit).await?;
        if cleared > 0 {
            warn!(
                file_path = %file_path,
                "cleared long-term archive replay marker after source reconciliation could not read the archive"
            );
        }
    }
    Ok(())
}

pub(crate) async fn advance_long_term_integrity_source_start_tx(
    tx: &mut SqliteConnection,
    retiring_archive_batch_id: i64,
    source_safe_start: NaiveDate,
) -> Result<()> {
    let pending_source_start = sqlx::query_scalar::<_, Option<String>>(
        "SELECT integrity_source_pending_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(&mut *tx)
    .await?
    .flatten()
    .map(|value| {
        NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|error| {
            anyhow!("pending long-term integrity source boundary is invalid ({value}): {error}")
        })
    })
    .transpose()?;
    let candidate = pending_source_start
        .map(|pending| pending.max(source_safe_start))
        .unwrap_or(source_safe_start);
    let candidate_start = candidate.to_string();
    let retained_source_blocks_boundary = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM archive_batches
            WHERE id <> ?1
              AND dataset IN ('codex_invocations', 'pool_upstream_request_attempts')
              AND status = 'completed'
              AND (coverage_start_at IS NULL OR coverage_start_at < ?2)
        )
        "#,
    )
    .bind(retiring_archive_batch_id)
    .bind(&candidate_start)
    .fetch_one(&mut *tx)
    .await?
        != 0;
    if retained_source_blocks_boundary {
        sqlx::query(
            "UPDATE long_term_stats_state SET integrity_source_pending_start_date = ?1, updated_at = datetime('now') WHERE id = ?2",
        )
        .bind(candidate_start)
        .bind(LONG_TERM_STATE_ID)
        .execute(&mut *tx)
        .await?;
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE long_term_stats_state
        SET integrity_source_start_date = CASE
            WHEN integrity_source_start_date IS NULL
              OR integrity_source_start_date < ?1 THEN ?1
            ELSE integrity_source_start_date
        END,
        integrity_source_pending_start_date = NULL,
        updated_at = datetime('now')
        WHERE id = ?2
        "#,
    )
    .bind(candidate_start)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn long_term_integrity_source_safe_start_for_archive_cleanup(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    coverage_end_at: Option<&str>,
) -> Result<Option<NaiveDate>> {
    match dataset {
        HOURLY_ROLLUP_DATASET_INVOCATIONS => {
            long_term_invocation_archive_safe_start(pool, file_path).await
        }
        "pool_upstream_request_attempts" => {
            long_term_attempt_archive_safe_start(pool, file_path, coverage_end_at).await
        }
        _ => Ok(None),
    }
}

pub(super) async fn long_term_invocation_archive_safe_start(
    pool: &Pool<Sqlite>,
    file_path: &str,
) -> Result<Option<NaiveDate>> {
    let archive_sha256 = load_long_term_archive_sha256(pool, file_path)
        .await?
        .context("completed invocation archive has no manifest sha256")?;
    let rows = load_long_term_source_timing_rows_from_archive(
        pool,
        file_path,
        &archive_sha256,
        "long-term-stats-cleanup-invocation-boundary",
    )
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let latest_effective_date = rows
        .iter()
        .map(|row| {
            long_term_source_effective_date(&row.occurred_at, row.t_total_ms).ok_or_else(|| {
                anyhow!(
                    "invocation archive has an unparseable timestamp for source-boundary verification: {}",
                    row.occurred_at
                )
            })
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .expect("non-empty invocation archive produces one effective date per source row");
    Ok(Some(long_term_source_safe_start_after_effective_date(
        latest_effective_date,
    )))
}

pub(super) async fn long_term_attempt_archive_safe_start(
    pool: &Pool<Sqlite>,
    file_path: &str,
    _coverage_end_at: Option<&str>,
) -> Result<Option<NaiveDate>> {
    let archive_sha256 = load_long_term_archive_sha256_for_dataset(
        pool,
        "pool_upstream_request_attempts",
        file_path,
    )
    .await?
    .context("completed attempt archive has no manifest sha256")?;
    ensure_long_term_archive_source_identity(
        pool,
        "pool_upstream_request_attempts",
        file_path,
        &archive_sha256,
    )
    .await?;
    let Some((archive_pool, cleanup)) = open_pool_upstream_request_attempt_archive_batch_pool(
        &ArchiveBatchPathRow::from_file_path(file_path.to_string()),
        "long-term-stats-cleanup-attempt-boundary",
    )
    .await?
    else {
        bail!("attempt archive is unavailable for long-term source-boundary verification");
    };
    let rows = sqlx::query_as::<_, LongTermArchiveAttemptRow>(
        r#"
        SELECT invoke_id, occurred_at, upstream_account_id
        FROM pool_upstream_request_attempts
        WHERE upstream_account_id IS NOT NULL
        ORDER BY id ASC
        "#,
    )
    .fetch_all(&archive_pool)
    .await;
    archive_pool.close().await;
    drop(cleanup);
    ensure_long_term_archive_source_identity(
        pool,
        "pool_upstream_request_attempts",
        file_path,
        &archive_sha256,
    )
    .await?;
    let pairs = rows?
        .into_iter()
        .filter_map(|row| {
            row.upstream_account_id
                .map(|_| (row.invoke_id, row.occurred_at))
        })
        .collect::<HashSet<_>>();
    if pairs.is_empty() {
        return Ok(None);
    }

    let (latest_effective_date, unmatched_pairs) =
        long_term_match_attempt_pairs_to_invocation_sources(pool, pairs).await?;
    if !unmatched_pairs.is_empty() {
        bail!(
            "{} attempt archive account mapping(s) have no readable invocation source",
            unmatched_pairs.len()
        );
    }
    let latest_effective_date = latest_effective_date.ok_or_else(|| {
        anyhow!("attempt archive account mappings have no parseable invocation timestamps")
    })?;
    Ok(Some(long_term_source_safe_start_after_effective_date(
        latest_effective_date,
    )))
}

pub(super) async fn load_long_term_source_timing_rows_from_archive(
    pool: &Pool<Sqlite>,
    file_path: &str,
    archive_sha256: &str,
    read_surface: &'static str,
) -> Result<Vec<LongTermSourceTimingRow>> {
    ensure_long_term_archive_source_identity(pool, "codex_invocations", file_path, archive_sha256)
        .await?;
    let Some((archive_pool, cleanup)) = open_invocation_archive_batch_pool(
        &ArchiveBatchPathRow::from_file_path(file_path.to_string()),
        read_surface,
    )
    .await?
    else {
        bail!("invocation archive is unavailable for long-term source-boundary verification");
    };
    let archive_columns = load_archive_table_columns(&archive_pool, "codex_invocations").await?;
    if !archive_columns.contains("occurred_at") {
        archive_pool.close().await;
        drop(cleanup);
        bail!("invocation archive has no occurred_at column for source-boundary verification");
    }
    let sql = long_term_source_timing_archive_query(&archive_columns);
    let rows = sqlx::query_as::<_, LongTermSourceTimingRow>(&sql)
        .fetch_all(&archive_pool)
        .await;
    archive_pool.close().await;
    drop(cleanup);
    ensure_long_term_archive_source_identity(pool, "codex_invocations", file_path, archive_sha256)
        .await?;
    Ok(rows?)
}

pub(super) async fn long_term_match_attempt_pairs_to_invocation_sources(
    pool: &Pool<Sqlite>,
    mut unmatched_pairs: HashSet<(String, String)>,
) -> Result<(Option<NaiveDate>, HashSet<(String, String)>)> {
    let mut latest_effective_date = None;
    let live_rows =
        load_long_term_source_timing_rows_for_pairs(pool, &unmatched_pairs, "t_total_ms").await?;
    record_long_term_matched_attempt_source_rows(
        &mut unmatched_pairs,
        &mut latest_effective_date,
        live_rows,
    )?;

    if unmatched_pairs.is_empty() {
        return Ok((latest_effective_date, unmatched_pairs));
    }
    let archive_paths = load_completed_invocation_archive_paths(pool).await?;
    for archive_path in archive_paths {
        if unmatched_pairs.is_empty() {
            break;
        }
        if !long_term_archive_may_contain_attempt_pairs(&archive_path, &unmatched_pairs) {
            continue;
        }
        let archive_sha256 = load_long_term_archive_sha256(pool, archive_path.file_path())
            .await?
            .context("completed invocation archive has no manifest sha256")?;
        ensure_long_term_archive_source_identity(
            pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
        let Some((archive_pool, cleanup)) = open_invocation_archive_batch_pool(
            &archive_path,
            "long-term-stats-cleanup-attempt-invocation-match",
        )
        .await?
        else {
            bail!("invocation archive is unavailable while verifying attempt account mappings");
        };
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        if !archive_columns.contains("invoke_id") || !archive_columns.contains("occurred_at") {
            archive_pool.close().await;
            drop(cleanup);
            bail!("invocation archive lacks keys required to verify attempt account mappings");
        }
        let t_total_ms_expression = long_term_legacy_column_expr(&archive_columns, "t_total_ms");
        let rows = load_long_term_source_timing_rows_for_pairs(
            &archive_pool,
            &unmatched_pairs,
            &t_total_ms_expression,
        )
        .await;
        archive_pool.close().await;
        drop(cleanup);
        ensure_long_term_archive_source_identity(
            pool,
            "codex_invocations",
            archive_path.file_path(),
            &archive_sha256,
        )
        .await?;
        record_long_term_matched_attempt_source_rows(
            &mut unmatched_pairs,
            &mut latest_effective_date,
            rows?,
        )?;
    }

    Ok((latest_effective_date, unmatched_pairs))
}

pub(super) async fn load_long_term_source_timing_rows_for_pairs(
    pool: &Pool<Sqlite>,
    pairs: &HashSet<(String, String)>,
    t_total_ms_expression: &str,
) -> Result<Vec<LongTermSourceTimingRow>> {
    const LONG_TERM_ATTEMPT_SOURCE_QUERY_BATCH_SIZE: usize = 400;

    let pairs = pairs.iter().collect::<Vec<_>>();
    let mut rows = Vec::new();
    for batch in pairs.chunks(LONG_TERM_ATTEMPT_SOURCE_QUERY_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new("SELECT invoke_id, occurred_at, ");
        query.push(t_total_ms_expression);
        query.push(" AS t_total_ms FROM codex_invocations WHERE ");
        for (index, (invoke_id, occurred_at)) in batch.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind((*invoke_id).clone())
                .push(" AND occurred_at = ")
                .push_bind((*occurred_at).clone())
                .push(")");
        }
        rows.extend(
            query
                .build_query_as::<LongTermSourceTimingRow>()
                .fetch_all(pool)
                .await?,
        );
    }
    Ok(rows)
}

pub(super) fn long_term_source_timing_archive_query(columns: &HashSet<String>) -> String {
    format!(
        "SELECT {} AS invoke_id, occurred_at, {} AS t_total_ms FROM codex_invocations",
        long_term_legacy_column_expr(columns, "invoke_id"),
        long_term_legacy_column_expr(columns, "t_total_ms"),
    )
}

pub(super) fn long_term_archive_may_contain_attempt_pairs(
    archive_path: &ArchiveBatchPathRow,
    pairs: &HashSet<(String, String)>,
) -> bool {
    let Some(start) = archive_path
        .coverage_start_at()
        .and_then(long_term_archive_end_date)
    else {
        return true;
    };
    let end = archive_path
        .coverage_end_at()
        .and_then(long_term_archive_end_date)
        .unwrap_or(start);
    pairs.iter().any(|(_, occurred_at)| {
        parse_long_term_timestamp_ms(occurred_at)
            .and_then(|timestamp| Shanghai.timestamp_millis_opt(timestamp).single())
            .map(|timestamp| {
                let date = timestamp.date_naive();
                date >= start && date <= end
            })
            .unwrap_or(true)
    })
}

pub(super) fn record_long_term_matched_attempt_source_rows(
    unmatched_pairs: &mut HashSet<(String, String)>,
    latest_effective_date: &mut Option<NaiveDate>,
    rows: Vec<LongTermSourceTimingRow>,
) -> Result<()> {
    let requested_pairs = unmatched_pairs.clone();
    for row in rows {
        let Some(invoke_id) = row.invoke_id else {
            continue;
        };
        let pair = (invoke_id, row.occurred_at.clone());
        if !requested_pairs.contains(&pair) {
            continue;
        }
        let date = long_term_source_effective_date(&row.occurred_at, row.t_total_ms).ok_or_else(
            || {
                anyhow!(
                    "matched attempt invocation source has an unparseable timestamp for source-boundary verification: {}",
                    row.occurred_at
                )
            },
        )?;
        unmatched_pairs.remove(&pair);
        *latest_effective_date =
            Some(latest_effective_date.map_or(date, |current| current.max(date)));
    }
    Ok(())
}

pub(super) fn long_term_source_effective_date(
    occurred_at: &str,
    t_total_ms: Option<f64>,
) -> Option<NaiveDate> {
    let start = parse_long_term_timestamp(occurred_at)?;
    let end_ms = t_total_ms
        .filter(|value| value.is_finite() && *value > 0.0)
        .and_then(|duration_ms| long_term_interval_end_ms(start, duration_ms))
        .unwrap_or(start.epoch_ms);
    Shanghai
        .timestamp_millis_opt(end_ms)
        .single()
        .map(|timestamp| timestamp.date_naive())
}

pub(super) fn long_term_integrity_totals_match(
    expected: LongTermIntegrityTotals,
    observed: LongTermIntegrityTotals,
) -> bool {
    expected.calls == observed.calls
        && expected.token_total == observed.token_total
        && (expected.cost_total - observed.cost_total).abs()
            <= 1e-6_f64.max(expected.cost_total.abs() * 1e-9)
}

pub(super) fn long_term_integrity_totals_are_empty(totals: LongTermIntegrityTotals) -> bool {
    totals.calls == 0 && totals.token_total == 0 && totals.cost_total.abs() <= 1e-12
}

pub(super) fn long_term_integrity_mismatch(
    date: NaiveDate,
    expected_daily: LongTermIntegrityTotals,
    expected_hourly: &HashMap<i64, LongTermIntegrityTotals>,
    observed_daily: LongTermIntegrityTotals,
    observed_hourly: &HashMap<i64, LongTermIntegrityTotals>,
) -> Option<LongTermIntegrityMismatch> {
    if !long_term_integrity_totals_match(expected_daily, observed_daily) {
        return Some(LongTermIntegrityMismatch {
            date,
            expected: expected_daily,
            observed: observed_daily,
            reason: format!(
                "daily overall differs: expected calls={}, tokens={}, cost={:.9}; observed calls={}, tokens={}, cost={:.9}",
                expected_daily.calls,
                expected_daily.token_total,
                expected_daily.cost_total,
                observed_daily.calls,
                observed_daily.token_total,
                observed_daily.cost_total,
            ),
        });
    }
    for (bucket_start_epoch, expected) in expected_hourly {
        let observed = observed_hourly
            .get(bucket_start_epoch)
            .copied()
            .unwrap_or_default();
        if !long_term_integrity_totals_match(*expected, observed) {
            return Some(LongTermIntegrityMismatch {
                date,
                expected: *expected,
                observed,
                reason: format!(
                    "hourly overall differs at {}: expected calls={}, tokens={}, cost={:.9}; observed calls={}, tokens={}, cost={:.9}",
                    bucket_start_epoch,
                    expected.calls,
                    expected.token_total,
                    expected.cost_total,
                    observed.calls,
                    observed.token_total,
                    observed.cost_total,
                ),
            });
        }
    }
    for (bucket_start_epoch, observed) in observed_hourly {
        if !expected_hourly.contains_key(bucket_start_epoch)
            && !long_term_integrity_totals_are_empty(*observed)
        {
            return Some(LongTermIntegrityMismatch {
                date,
                expected: LongTermIntegrityTotals::default(),
                observed: *observed,
                reason: format!(
                    "hourly overall has an unexpected non-empty bucket at {bucket_start_epoch}"
                ),
            });
        }
    }
    None
}

pub(super) async fn long_term_integrity_oracle_available(pool: &Pool<Sqlite>) -> Result<bool> {
    let columns = load_sqlite_table_columns(pool, "invocation_rollup_hourly").await?;
    Ok([
        "terminal_count",
        "terminal_tokens",
        "terminal_cost",
        "terminal_proof_complete",
    ]
    .into_iter()
    .all(|column| columns.contains(column)))
}

pub(super) async fn load_long_term_integrity_oracle(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
) -> Result<Option<LongTermIntegrityOracle>> {
    if !long_term_integrity_oracle_available(pool).await? {
        return Ok(None);
    }
    let Some((start_epoch, end_epoch)) = long_term_day_epoch_bounds(date) else {
        return Ok(None);
    };
    let has_untrusted_hour = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM invocation_rollup_hourly
            WHERE bucket_start_epoch >= ?1
              AND bucket_start_epoch < ?2
              AND terminal_proof_complete <> 1
        )
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_one(pool)
    .await?
        != 0;
    if has_untrusted_hour {
        return Ok(None);
    }
    let rows = sqlx::query_as::<_, LongTermIntegrityHourRow>(
        r#"
        SELECT bucket_start_epoch,
               COALESCE(SUM(terminal_count), 0) AS calls,
               COALESCE(SUM(terminal_tokens), 0) AS token_total,
               COALESCE(SUM(terminal_cost), 0.0) AS cost_total
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2
        GROUP BY bucket_start_epoch
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let mut daily = LongTermIntegrityTotals::default();
    let mut hourly = HashMap::new();
    for row in rows {
        let totals = LongTermIntegrityTotals {
            calls: row.calls,
            token_total: row.token_total,
            cost_total: row.cost_total,
        };
        daily.calls += totals.calls;
        daily.token_total += totals.token_total;
        daily.cost_total += totals.cost_total;
        hourly.insert(row.bucket_start_epoch, totals);
    }
    Ok(Some(LongTermIntegrityOracle {
        date,
        daily,
        hourly,
    }))
}

pub(super) fn long_term_candidate_integrity(
    date: NaiveDate,
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
) -> (
    LongTermIntegrityTotals,
    HashMap<i64, LongTermIntegrityTotals>,
) {
    let date_string = date.to_string();
    let daily_totals = daily
        .iter()
        .filter(|((bucket_date, dimension, _), _)| {
            bucket_date == &date_string && dimension == "overall"
        })
        .fold(
            LongTermIntegrityTotals::default(),
            |mut totals, (_, bucket)| {
                totals.calls += bucket.accumulator.calls;
                totals.token_total += bucket.accumulator.token_total;
                totals.cost_total += bucket.accumulator.cost_total;
                totals
            },
        );
    let mut hourly_totals = HashMap::new();
    for ((bucket_start_epoch, dimension, _), bucket) in hourly {
        if dimension != "overall" || long_term_bucket_date(*bucket_start_epoch) != Some(date) {
            continue;
        }
        let totals = hourly_totals
            .entry(*bucket_start_epoch)
            .or_insert_with(LongTermIntegrityTotals::default);
        totals.calls += bucket.accumulator.calls;
        totals.token_total += bucket.accumulator.token_total;
        totals.cost_total += bucket.accumulator.cost_total;
    }
    (daily_totals, hourly_totals)
}

pub(super) fn remove_long_term_candidate_dates(
    hourly: &mut HashMap<(i64, String, String), LongTermBucket>,
    daily: &mut HashMap<(String, String, String), LongTermBucket>,
    dates: &HashSet<NaiveDate>,
) {
    if dates.is_empty() {
        return;
    }
    hourly.retain(|(bucket_start, _, _), _| {
        Shanghai
            .timestamp_opt(*bucket_start, 0)
            .single()
            .map(|value| !dates.contains(&value.date_naive()))
            .unwrap_or(true)
    });
    daily.retain(|(date, _, _), _| {
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map(|value| !dates.contains(&value))
            .unwrap_or(true)
    });
}

pub(super) async fn enqueue_long_term_integrity_mismatch(
    pool: &Pool<Sqlite>,
    mismatch: &LongTermIntegrityMismatch,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        r#"
        INSERT INTO long_term_stats_repair_queue (
            stats_date,
            expected_calls,
            expected_token_total,
            expected_cost_total,
            observed_calls,
            observed_token_total,
            observed_cost_total,
            last_error
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(stats_date) DO UPDATE SET
            expected_calls = excluded.expected_calls,
            expected_token_total = excluded.expected_token_total,
            expected_cost_total = excluded.expected_cost_total,
            observed_calls = excluded.observed_calls,
            observed_token_total = excluded.observed_token_total,
            observed_cost_total = excluded.observed_cost_total,
            last_error = excluded.last_error,
            updated_at = datetime('now')
        "#,
    )
    .bind(mismatch.date.to_string())
    .bind(mismatch.expected.calls)
    .bind(mismatch.expected.token_total)
    .bind(mismatch.expected.cost_total)
    .bind(mismatch.observed.calls)
    .bind(mismatch.observed.token_total)
    .bind(mismatch.observed.cost_total)
    .bind(&mismatch.reason)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

pub(super) async fn load_long_term_reconciliation_mismatches(
    pool: &Pool<Sqlite>,
    invalidated_bucket_start_epochs: &[i64],
    reconstructable_start: NaiveDate,
) -> Result<Vec<LongTermIntegrityMismatch>> {
    let dates = invalidated_bucket_start_epochs
        .iter()
        .filter_map(|bucket_start_epoch| long_term_bucket_date(*bucket_start_epoch))
        .filter(|date| *date >= reconstructable_start)
        .collect::<BTreeSet<_>>();
    let mut mismatches = Vec::with_capacity(dates.len());
    for date in dates {
        let Some((start_epoch, end_epoch)) = long_term_day_epoch_bounds(date) else {
            continue;
        };
        // The canonical values remain useful as the durable expectation for a queued repair,
        // even after their proof bit is revoked. A later source reconciliation must restore the
        // proof before the repair path is allowed to replace the long-term rows.
        let expected = sqlx::query_as::<_, (i64, i64, f64)>(
            r#"
            SELECT COALESCE(SUM(terminal_count), 0),
                   COALESCE(SUM(terminal_tokens), 0),
                   COALESCE(SUM(terminal_cost), 0.0)
            FROM invocation_rollup_hourly
            WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2
            "#,
        )
        .bind(start_epoch)
        .bind(end_epoch)
        .fetch_one(pool)
        .await?;
        let observed = sqlx::query_as::<_, (i64, i64, f64)>(
            r#"
            SELECT COALESCE(SUM(calls), 0),
                   COALESCE(SUM(token_total), 0),
                   COALESCE(SUM(cost_total), 0.0)
            FROM long_term_usage_daily
            WHERE stats_date = ?1 AND dimension = 'overall'
            "#,
        )
        .bind(date.to_string())
        .fetch_one(pool)
        .await?;
        mismatches.push(LongTermIntegrityMismatch {
            date,
            expected: LongTermIntegrityTotals {
                calls: expected.0,
                token_total: expected.1,
                cost_total: expected.2,
            },
            observed: LongTermIntegrityTotals {
                calls: observed.0,
                token_total: observed.1,
                cost_total: observed.2,
            },
            reason: "complete invocation source reconciliation disagreed with canonical hourly terminal totals".to_string(),
        });
    }
    Ok(mismatches)
}

pub(super) async fn next_due_long_term_repair_date(
    pool: &Pool<Sqlite>,
    reconstructable_start: NaiveDate,
) -> Result<Option<NaiveDate>> {
    let date = sqlx::query_scalar::<_, String>(
        r#"
        SELECT stats_date
        FROM long_term_stats_repair_queue
        WHERE stats_date >= ?1
          AND datetime(next_retry_at) <= datetime('now')
        ORDER BY datetime(next_retry_at) ASC, stats_date ASC
        LIMIT 1
        "#,
    )
    .bind(reconstructable_start.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(date.and_then(|value| NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok()))
}

pub(super) async fn queued_long_term_repair_mismatch(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
    observed: LongTermIntegrityTotals,
    reason: impl Into<String>,
) -> Result<Option<LongTermIntegrityMismatch>> {
    let expected = sqlx::query_as::<_, (i64, i64, f64)>(
        "SELECT expected_calls, expected_token_total, expected_cost_total FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(expected.map(
        |(calls, token_total, cost_total)| LongTermIntegrityMismatch {
            date,
            expected: LongTermIntegrityTotals {
                calls,
                token_total,
                cost_total,
            },
            observed,
            reason: reason.into(),
        },
    ))
}

pub(super) async fn long_term_integrity_audit_due(pool: &Pool<Sqlite>) -> Result<bool> {
    let due = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT CASE
            WHEN last_integrity_audit_at IS NULL THEN 1
            WHEN datetime(last_integrity_audit_at) <= datetime('now', ?1) THEN 1
            ELSE 0
        END
        FROM long_term_stats_state
        WHERE id = ?2
        "#,
    )
    .bind(format!(
        "-{} seconds",
        LONG_TERM_INTEGRITY_AUDIT_INTERVAL_SECS
    ))
    .bind(LONG_TERM_STATE_ID)
    .fetch_optional(pool)
    .await?
    .unwrap_or(1);
    Ok(due != 0)
}

pub(super) async fn mark_long_term_integrity_audit(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_stats_state SET last_integrity_audit_at = datetime('now') WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

pub(super) async fn audit_long_term_integrity(
    pool: &Pool<Sqlite>,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<LongTermIntegrityMismatch>> {
    if start_date > end_date || !long_term_integrity_oracle_available(pool).await? {
        return Ok(Vec::new());
    }
    let Some((start_epoch, _)) = long_term_day_epoch_bounds(start_date) else {
        return Ok(Vec::new());
    };
    let Some((_, end_epoch)) = long_term_day_epoch_bounds(end_date) else {
        return Ok(Vec::new());
    };
    let untrusted_dates = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT DISTINCT bucket_start_epoch
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
          AND terminal_proof_complete <> 1
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?
    .into_iter()
    .filter_map(long_term_bucket_date)
    .collect::<HashSet<_>>();
    let expected_rows = sqlx::query_as::<_, LongTermIntegrityHourRow>(
        r#"
        SELECT bucket_start_epoch,
               COALESCE(SUM(terminal_count), 0) AS calls,
               COALESCE(SUM(terminal_tokens), 0) AS token_total,
               COALESCE(SUM(terminal_cost), 0.0) AS cost_total
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
          AND terminal_proof_complete = 1
        GROUP BY bucket_start_epoch
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let materialized_daily_dates = sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT stats_date
        FROM long_term_usage_daily
        WHERE stats_date >= ?1 AND stats_date <= ?2
        "#,
    )
    .bind(start_date.to_string())
    .bind(end_date.to_string())
    .fetch_all(pool)
    .await?;
    let materialized_hourly_buckets = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT DISTINCT bucket_start_epoch
        FROM long_term_usage_hourly
        WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let materialized_nonempty_daily_dates = sqlx::query_scalar::<_, String>(
        r#"
        SELECT DISTINCT stats_date
        FROM long_term_usage_daily
        WHERE stats_date >= ?1 AND stats_date <= ?2
          AND (calls <> 0 OR token_total <> 0 OR ABS(cost_total) > 1e-12)
        "#,
    )
    .bind(start_date.to_string())
    .bind(end_date.to_string())
    .fetch_all(pool)
    .await?;
    let materialized_nonempty_hourly_buckets = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT DISTINCT bucket_start_epoch
        FROM long_term_usage_hourly
        WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2
          AND (calls <> 0 OR token_total <> 0 OR ABS(cost_total) > 1e-12)
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let actual_hourly_rows = sqlx::query_as::<_, LongTermIntegrityHourRow>(
        r#"
        SELECT bucket_start_epoch,
               COALESCE(SUM(calls), 0) AS calls,
               COALESCE(SUM(token_total), 0) AS token_total,
               COALESCE(SUM(cost_total), 0.0) AS cost_total
        FROM long_term_usage_hourly
        WHERE dimension = 'overall'
          AND bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
        GROUP BY bucket_start_epoch
        "#,
    )
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await?;
    let actual_daily_rows = sqlx::query_as::<_, (String, i64, i64, f64)>(
        r#"
        SELECT stats_date,
               COALESCE(SUM(calls), 0),
               COALESCE(SUM(token_total), 0),
               COALESCE(SUM(cost_total), 0.0)
        FROM long_term_usage_daily
        WHERE dimension = 'overall' AND stats_date >= ?1 AND stats_date <= ?2
        GROUP BY stats_date
        "#,
    )
    .bind(start_date.to_string())
    .bind(end_date.to_string())
    .fetch_all(pool)
    .await?;

    let mut expected_by_date: HashMap<NaiveDate, HashMap<i64, LongTermIntegrityTotals>> =
        HashMap::new();
    for row in expected_rows {
        let Some(date) = long_term_bucket_date(row.bucket_start_epoch) else {
            continue;
        };
        expected_by_date.entry(date).or_default().insert(
            row.bucket_start_epoch,
            LongTermIntegrityTotals {
                calls: row.calls,
                token_total: row.token_total,
                cost_total: row.cost_total,
            },
        );
    }
    let mut actual_hourly_by_date: HashMap<NaiveDate, HashMap<i64, LongTermIntegrityTotals>> =
        HashMap::new();
    for row in actual_hourly_rows {
        let Some(date) = long_term_bucket_date(row.bucket_start_epoch) else {
            continue;
        };
        actual_hourly_by_date.entry(date).or_default().insert(
            row.bucket_start_epoch,
            LongTermIntegrityTotals {
                calls: row.calls,
                token_total: row.token_total,
                cost_total: row.cost_total,
            },
        );
    }
    let actual_daily_by_date = actual_daily_rows
        .into_iter()
        .filter_map(|(date, calls, token_total, cost_total)| {
            NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .ok()
                .map(|date| {
                    (
                        date,
                        LongTermIntegrityTotals {
                            calls,
                            token_total,
                            cost_total,
                        },
                    )
                })
        })
        .collect::<HashMap<_, _>>();
    let mut materialized_dates = materialized_daily_dates
        .into_iter()
        .filter_map(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok())
        .collect::<HashSet<_>>();
    materialized_dates.extend(
        materialized_hourly_buckets
            .into_iter()
            .filter_map(long_term_bucket_date),
    );
    let mut materialized_nonempty_dates = materialized_nonempty_daily_dates
        .into_iter()
        .filter_map(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok())
        .collect::<HashSet<_>>();
    materialized_nonempty_dates.extend(
        materialized_nonempty_hourly_buckets
            .into_iter()
            .filter_map(long_term_bucket_date),
    );

    // Audit both sides of the comparison. A non-empty materialized day with no canonical
    // hourly rows is corrupt, but a zero-total wall-time continuation from a prior-day call is
    // legitimate and cannot be disproved by a start-hour canonical rollup.
    let mut audit_dates = expected_by_date
        .keys()
        .chain(actual_hourly_by_date.keys())
        .chain(actual_daily_by_date.keys())
        .chain(materialized_dates.iter())
        .copied()
        .collect::<Vec<_>>();
    audit_dates.sort_unstable();
    audit_dates.dedup();
    let empty_hourly = HashMap::new();
    let mut mismatches = Vec::new();
    for date in audit_dates {
        if untrusted_dates.contains(&date) {
            continue;
        }
        let expected_hourly = expected_by_date.get(&date).unwrap_or(&empty_hourly);
        let expected_daily = expected_hourly.values().fold(
            LongTermIntegrityTotals::default(),
            |mut totals, value| {
                totals.calls += value.calls;
                totals.token_total += value.token_total;
                totals.cost_total += value.cost_total;
                totals
            },
        );
        if long_term_integrity_totals_are_empty(expected_daily)
            && materialized_dates.contains(&date)
            && materialized_nonempty_dates.contains(&date)
        {
            mismatches.push(LongTermIntegrityMismatch {
                date,
                expected: LongTermIntegrityTotals::default(),
                observed: actual_daily_by_date.get(&date).copied().unwrap_or_default(),
                reason: "canonical terminal totals are empty but materialized rows remain in one or more dimensions".to_string(),
            });
            continue;
        }
        if let Some(mismatch) = long_term_integrity_mismatch(
            date,
            expected_daily,
            expected_hourly,
            actual_daily_by_date.get(&date).copied().unwrap_or_default(),
            actual_hourly_by_date.get(&date).unwrap_or(&empty_hourly),
        ) {
            mismatches.push(mismatch);
        }
    }
    Ok(mismatches)
}

pub(super) fn long_term_repair_backoff_secs(attempts: i64) -> i64 {
    let index = attempts.saturating_sub(1) as usize;
    LONG_TERM_REPAIR_BACKOFF_SECS[index.min(LONG_TERM_REPAIR_BACKOFF_SECS.len() - 1)]
}

pub(super) async fn schedule_long_term_repair_retry(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    mismatch: &LongTermIntegrityMismatch,
) -> Result<()> {
    let attempts = sqlx::query_scalar::<_, i64>(
        "SELECT attempts FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(mismatch.date.to_string())
    .fetch_optional(&mut **tx)
    .await?
    .unwrap_or(0)
        + 1;
    let retry_modifier = format!("+{} seconds", long_term_repair_backoff_secs(attempts));
    sqlx::query(
        r#"
        INSERT INTO long_term_stats_repair_queue (
            stats_date,
            expected_calls,
            expected_token_total,
            expected_cost_total,
            observed_calls,
            observed_token_total,
            observed_cost_total,
            attempts,
            next_retry_at,
            last_error
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now', ?9), ?10)
        ON CONFLICT(stats_date) DO UPDATE SET
            expected_calls = excluded.expected_calls,
            expected_token_total = excluded.expected_token_total,
            expected_cost_total = excluded.expected_cost_total,
            observed_calls = excluded.observed_calls,
            observed_token_total = excluded.observed_token_total,
            observed_cost_total = excluded.observed_cost_total,
            attempts = excluded.attempts,
            next_retry_at = excluded.next_retry_at,
            last_error = excluded.last_error,
            updated_at = datetime('now')
        "#,
    )
    .bind(mismatch.date.to_string())
    .bind(mismatch.expected.calls)
    .bind(mismatch.expected.token_total)
    .bind(mismatch.expected.cost_total)
    .bind(mismatch.observed.calls)
    .bind(mismatch.observed.token_total)
    .bind(mismatch.observed.cost_total)
    .bind(attempts)
    .bind(retry_modifier)
    .bind(&mismatch.reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
