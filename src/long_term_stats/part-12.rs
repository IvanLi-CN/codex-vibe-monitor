async fn apply_long_term_projection_incremental_with_runtime_and_control(
    pool: &Pool<Sqlite>,
    runtime: &Arc<Mutex<LongTermProjectionRuntime>>,
    batch: LongTermProjectionIncrementalBatch<'_>,
    next_cursor: i64,
    event_count: usize,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermProjectionIncrementalOutcome> {
    validate_long_term_projection_incremental_batch(&batch)?;
    let mut dates = HashSet::new();
    for segment in batch.segments {
        dates.extend(long_term_projection_interval_dates(segment));
    }
    for bucket in batch.daily.values() {
        if let Some(date) = bucket.stats_date.as_ref() {
            dates.insert(date.clone());
        }
    }
    let (mut interval_index, loaded_dates) =
        load_long_term_projection_incremental_interval_index(pool, runtime, &dates).await?;
    let persisted_segment_ids =
        load_long_term_projection_interval_state_ids(pool, batch.segments).await?;
    for segment in batch
        .segments
        .iter()
        .filter(|segment| !persisted_segment_ids.contains(&segment.invocation_row_id))
    {
        let state = LongTermProjectionIntervalStateRow {
            invocation_row_id: segment.invocation_row_id,
            model_series_key: segment.model_series_key.clone(),
            upstream_series_key: segment.upstream_series_key.clone(),
            interval_start_ms: segment.interval_start_ms,
            interval_end_ms: segment.interval_end_ms,
        };
        for date in &dates {
            if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                add_long_term_projection_interval_state_for_date(&mut interval_index, date, &state);
            }
        }
    }
    let (mut tx, permit) = control.begin(pool).await?;
    if long_term_projection_incremental_dates_are_dirty(&mut tx, &dates).await? {
        drop(tx);
        drop(permit);
        return Ok(LongTermProjectionIncrementalOutcome::RebuildRequired);
    }
    upsert_long_term_projection_interval_segments_in_transaction(&mut tx, batch.segments).await?;
    for bucket in batch.hourly.values() {
        let key = projection_interval_key(
            "hourly",
            bucket.bucket_start_epoch.to_string(),
            bucket.dimension.clone(),
            bucket.series_key.clone(),
        );
        merge_long_term_projection_rollup(
            &mut tx,
            "long_term_usage_hourly",
            "bucket_start_epoch",
            bucket.bucket_start_epoch.to_string(),
            bucket,
            interval_index.get(&key),
        )
        .await?;
    }
    for bucket in batch.daily.values() {
        let Some(date) = bucket.stats_date.clone() else {
            continue;
        };
        let key = projection_interval_key(
            "daily",
            date.clone(),
            bucket.dimension.clone(),
            bucket.series_key.clone(),
        );
        merge_long_term_projection_rollup(
            &mut tx,
            "long_term_usage_daily",
            "stats_date",
            date,
            bucket,
            interval_index.get(&key),
        )
        .await?;
    }
    persist_long_term_projection_incremental_state(&mut tx, next_cursor, event_count, batch.daily)
        .await?;
    control.commit(tx, permit).await?;
    let mut runtime = runtime.lock().await;
    runtime.interval_index = interval_index;
    runtime.loaded_interval_dates = loaded_dates;
    Ok(LongTermProjectionIncrementalOutcome::Published)
}

fn validate_long_term_projection_incremental_batch(
    batch: &LongTermProjectionIncrementalBatch<'_>,
) -> Result<()> {
    let mutation_rows = batch.hourly.len() + batch.daily.len() + batch.segments.len();
    if mutation_rows > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS {
        bail!(
            "long-term projection incremental batch has {mutation_rows} writes; maximum is {LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS}"
        );
    }
    Ok(())
}

async fn load_long_term_projection_incremental_interval_index(
    pool: &Pool<Sqlite>,
    runtime: &Arc<Mutex<LongTermProjectionRuntime>>,
    dates: &HashSet<String>,
) -> Result<(
    HashMap<LongTermProjectionIntervalKey, LongTermProjectionIntervalUnion>,
    HashSet<String>,
)> {
    let (mut interval_index, mut loaded_dates) = {
        let runtime = runtime.lock().await;
        (
            runtime.interval_index.clone(),
            runtime.loaded_interval_dates.clone(),
        )
    };
    let missing_dates = dates
        .difference(&loaded_dates)
        .cloned()
        .collect::<HashSet<_>>();
    if !missing_dates.is_empty() {
        interval_index
            .extend(load_long_term_projection_interval_index(pool, &missing_dates).await?);
        loaded_dates.extend(missing_dates);
    }
    Ok((interval_index, loaded_dates))
}

async fn persist_long_term_projection_incremental_state(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    next_cursor: i64,
    event_count: usize,
    daily: &HashMap<(String, String, String), LongTermBucket>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO long_term_projection_state (consumer, cursor_row_id, last_flush_at, last_error) VALUES (?1, ?2, datetime('now'), NULL) ON CONFLICT(consumer) DO UPDATE SET cursor_row_id = excluded.cursor_row_id, last_flush_at = excluded.last_flush_at, last_error = NULL, updated_at = datetime('now')",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .bind(next_cursor)
    .execute(&mut **transaction)
    .await?;
    let statistics_start_date = daily
        .values()
        .filter_map(|bucket| bucket.stats_date.as_deref())
        .min()
        .map(str::to_string);
    sqlx::query(
        "UPDATE long_term_stats_state SET status = CASE WHEN ?1 > 0 AND status <> ?2 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets) THEN ?3 ELSE status END, statistics_start_date = CASE WHEN ?4 IS NULL THEN statistics_start_date WHEN statistics_start_date IS NULL OR ?4 < statistics_start_date THEN ?4 ELSE statistics_start_date END, processed_rows = processed_rows + ?1, total_rows = total_rows + ?1, last_error = CASE WHEN ?1 > 0 AND status <> ?2 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets) THEN NULL ELSE last_error END, updated_at = datetime('now') WHERE id = ?5",
    )
    .bind(event_count as i64)
    .bind(LONG_TERM_STATUS_ERROR)
    .bind(LONG_TERM_STATUS_READY)
    .bind(statistics_start_date)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn ensure_long_term_projection_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_long_term_projection_state_schema(pool).await?;
    ensure_long_term_projection_repair_schema(pool).await?;
    ensure_long_term_projection_publication_schema(pool).await?;
    ensure_long_term_projection_archive_schema(pool).await?;
    ensure_long_term_projection_interval_schema(pool).await
}

async fn ensure_long_term_projection_state_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_long_term_projection_schema_statement(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_state (
            consumer TEXT PRIMARY KEY, cursor_row_id INTEGER NOT NULL DEFAULT 0, last_flush_at TEXT,
            last_daily_verify_at TEXT, daily_verify_pending INTEGER NOT NULL DEFAULT 0,
            daily_verify_bucket_date TEXT, last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        "failed to ensure long-term projection state table",
    )
    .await?;
    for statement in [
        "ALTER TABLE long_term_projection_state ADD COLUMN last_daily_verify_at TEXT",
        "ALTER TABLE long_term_projection_state ADD COLUMN daily_verify_pending INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE long_term_projection_state ADD COLUMN daily_verify_bucket_date TEXT",
    ] {
        add_long_term_projection_schema_column(pool, statement).await?;
    }
    Ok(())
}

async fn ensure_long_term_projection_repair_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_long_term_projection_schema_statement(pool, r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_dirty_buckets (
            bucket_date TEXT PRIMARY KEY, repair_reason TEXT NOT NULL,
            generation INTEGER NOT NULL DEFAULT 1, queued_at TEXT NOT NULL DEFAULT (datetime('now')),
            next_attempt_at TEXT, updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#, "failed to ensure long-term projection dirty bucket table").await?;
    for statement in [
        "ALTER TABLE long_term_projection_dirty_buckets ADD COLUMN next_attempt_at TEXT",
        "ALTER TABLE long_term_projection_dirty_buckets ADD COLUMN generation INTEGER NOT NULL DEFAULT 1",
    ] {
        add_long_term_projection_schema_column(pool, statement).await?;
    }
    ensure_long_term_projection_schema_statement(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_bucket_state (
            bucket_date TEXT PRIMARY KEY, interval_baseline_ready INTEGER NOT NULL DEFAULT 0,
            active_daily_backup_token TEXT, publication_token TEXT, publication_generation INTEGER,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        "failed to ensure long-term projection bucket state table",
    )
    .await?;
    for statement in [
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN active_daily_backup_token TEXT",
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN publication_token TEXT",
        "ALTER TABLE long_term_projection_bucket_state ADD COLUMN publication_generation INTEGER",
    ] {
        add_long_term_projection_schema_column(pool, statement).await?;
    }
    Ok(())
}

async fn ensure_long_term_projection_publication_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_long_term_projection_schema_statement(pool, r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_daily_backups (
            rebuild_token TEXT NOT NULL, stats_date TEXT NOT NULL, dimension TEXT NOT NULL,
            series_key TEXT NOT NULL, display_name TEXT NOT NULL, reasoning_effort TEXT NOT NULL DEFAULT '',
            calls INTEGER NOT NULL DEFAULT 0, token_total INTEGER NOT NULL DEFAULT 0,
            token_samples INTEGER NOT NULL DEFAULT 0, cost_total REAL NOT NULL DEFAULT 0,
            cost_samples INTEGER NOT NULL DEFAULT 0, usage_time_ms REAL NOT NULL DEFAULT 0,
            usage_time_samples INTEGER NOT NULL DEFAULT 0, wall_time_ms REAL NOT NULL DEFAULT 0,
            wall_time_samples INTEGER NOT NULL DEFAULT 0, output_tokens_total INTEGER NOT NULL DEFAULT 0,
            stream_duration_ms REAL NOT NULL DEFAULT 0, output_speed_samples INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0, first_byte_samples INTEGER NOT NULL DEFAULT 0,
            response_sum_ms REAL NOT NULL DEFAULT 0, response_samples INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (rebuild_token, stats_date, dimension, series_key)
        )
        "#, "failed to ensure long-term projection daily backup table").await?;
    ensure_long_term_projection_schema_statement(pool,
        "CREATE INDEX IF NOT EXISTS idx_long_term_projection_daily_backups_token_date ON long_term_projection_daily_backups (rebuild_token, stats_date)",
        "failed to ensure long-term projection daily backup index").await?;
    ensure_long_term_projection_schema_statement(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_date_publications (
            publication_token TEXT PRIMARY KEY, published INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        "failed to ensure long-term projection date publication table",
    )
    .await?;
    ensure_long_term_projection_schema_statement(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_daily_backup_claims (
            bucket_date TEXT PRIMARY KEY, rebuild_token TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        "failed to ensure long-term projection daily backup claim table",
    )
    .await
}

async fn ensure_long_term_projection_archive_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_long_term_projection_schema_statement(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS long_term_projection_archive_compatibility (
            file_path TEXT PRIMARY KEY, archive_sha256 TEXT NOT NULL, file_fingerprint TEXT,
            has_legacy_crossing INTEGER NOT NULL, legacy_max_duration_ms REAL,
            legacy_min_occurred_at TEXT, has_rfc3339 INTEGER NOT NULL,
            rfc3339_max_duration_ms REAL, rfc3339_min_occurred_at TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        "failed to ensure long-term projection archive compatibility table",
    )
    .await?;
    for statement in [
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN legacy_max_duration_ms REAL",
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN legacy_min_occurred_at TEXT",
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN file_fingerprint TEXT",
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN rfc3339_max_duration_ms REAL",
        "ALTER TABLE long_term_projection_archive_compatibility ADD COLUMN rfc3339_min_occurred_at TEXT",
    ] {
        add_long_term_projection_schema_column(pool, statement).await?;
    }
    Ok(())
}

async fn ensure_long_term_projection_interval_schema(pool: &Pool<Sqlite>) -> Result<()> {
    for (statement, context) in [
        (
            r#"CREATE TABLE IF NOT EXISTS long_term_projection_intervals (
            bucket_kind TEXT NOT NULL, bucket_date TEXT NOT NULL, bucket_key TEXT NOT NULL,
            dimension TEXT NOT NULL, series_key TEXT NOT NULL, invocation_row_id INTEGER NOT NULL,
            interval_start_ms INTEGER NOT NULL, interval_end_ms INTEGER NOT NULL,
            PRIMARY KEY (bucket_kind, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms))"#,
            "failed to ensure long-term projection interval table",
        ),
        (
            "CREATE INDEX IF NOT EXISTS idx_long_term_projection_intervals_date ON long_term_projection_intervals (bucket_date, bucket_kind)",
            "failed to ensure long-term projection interval date index",
        ),
        (
            "CREATE INDEX IF NOT EXISTS idx_long_term_projection_intervals_invocation ON long_term_projection_intervals (invocation_row_id)",
            "failed to ensure long-term projection interval invocation index",
        ),
        (
            r#"CREATE TABLE IF NOT EXISTS long_term_projection_interval_state (
            invocation_row_id INTEGER PRIMARY KEY, model_series_key TEXT NOT NULL,
            upstream_series_key TEXT NOT NULL, interval_start_ms INTEGER NOT NULL, interval_end_ms INTEGER NOT NULL)"#,
            "failed to ensure long-term projection canonical interval state table",
        ),
        (
            "CREATE INDEX IF NOT EXISTS idx_long_term_projection_interval_state_end_start ON long_term_projection_interval_state (interval_end_ms, interval_start_ms)",
            "failed to ensure long-term projection canonical interval range index",
        ),
        (
            r#"CREATE TABLE IF NOT EXISTS long_term_projection_interval_suppressions (
            invocation_row_id INTEGER NOT NULL, bucket_date TEXT NOT NULL,
            PRIMARY KEY (invocation_row_id, bucket_date))"#,
            "failed to ensure long-term projection interval suppression table",
        ),
        (
            "CREATE INDEX IF NOT EXISTS idx_long_term_projection_interval_suppressions_date ON long_term_projection_interval_suppressions (bucket_date, invocation_row_id)",
            "failed to ensure long-term projection interval suppression date index",
        ),
        (
            r#"CREATE TABLE IF NOT EXISTS long_term_projection_rebuild_members (
            rebuild_token TEXT NOT NULL, invocation_row_id INTEGER NOT NULL,
            PRIMARY KEY (rebuild_token, invocation_row_id))"#,
            "failed to ensure long-term projection rebuild membership table",
        ),
    ] {
        ensure_long_term_projection_schema_statement(pool, statement, context).await?;
    }
    ensure_long_term_projection_schema_statement(
        pool,
        "DROP INDEX IF EXISTS idx_long_term_projection_interval_state_range",
        "failed to replace long-term projection interval range index",
    )
    .await
}

async fn ensure_long_term_projection_schema_statement(
    pool: &Pool<Sqlite>,
    statement: &str,
    context: &str,
) -> Result<()> {
    sqlx::query(statement)
        .execute(pool)
        .await
        .with_context(|| context.to_string())?;
    Ok(())
}

async fn add_long_term_projection_schema_column(
    pool: &Pool<Sqlite>,
    statement: &str,
) -> Result<()> {
    match sqlx::query(statement).execute(pool).await {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("duplicate column name") => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn load_long_term_rollups(
    pool: &Pool<Sqlite>,
    range: LongTermRange,
    dimension: &str,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<LongTermRollupRow>> {
    let table = "active_daily";
    let sql = format!(
        "{} SELECT MAX(d.stats_date) AS bucket_or_date, d.dimension, d.series_key, COALESCE((SELECT account.display_name FROM pool_upstream_accounts account WHERE d.dimension = 'upstream' AND d.series_key LIKE 'account:%' AND account.id = CAST(substr(d.series_key, 9) AS INTEGER) LIMIT 1), (SELECT latest.display_name FROM {table} latest WHERE latest.dimension = ?1 AND latest.series_key = d.series_key AND latest.reasoning_effort = d.reasoning_effort AND latest.stats_date BETWEEN ?2 AND ?3 ORDER BY latest.stats_date DESC LIMIT 1)) AS display_name, d.reasoning_effort, SUM(d.calls) AS calls, SUM(d.token_total) AS token_total, SUM(d.token_samples) AS token_samples, SUM(d.cost_total) AS cost_total, SUM(d.cost_samples) AS cost_samples, SUM(d.usage_time_ms) AS usage_time_ms, SUM(d.usage_time_samples) AS usage_time_samples, SUM(d.wall_time_ms) AS wall_time_ms, SUM(d.wall_time_samples) AS wall_time_samples, SUM(d.output_tokens_total) AS output_tokens_total, SUM(d.stream_duration_ms) AS stream_duration_ms, SUM(d.output_speed_samples) AS output_speed_samples, SUM(d.first_byte_sum_ms) AS first_byte_sum_ms, SUM(d.first_byte_samples) AS first_byte_samples, SUM(d.response_sum_ms) AS response_sum_ms, SUM(d.response_samples) AS response_samples FROM {table} d WHERE d.dimension = ?1 AND d.stats_date BETWEEN ?2 AND ?3 GROUP BY d.series_key, d.reasoning_effort",
        long_term_projection_active_daily_cte(),
    );
    let _ = range;
    Ok(sqlx::query_as::<_, LongTermRollupRow>(&sql)
        .bind(dimension)
        .bind(start_date)
        .bind(end_date)
        .fetch_all(pool)
        .await?)
}

async fn load_long_term_daily_rows(
    pool: &Pool<Sqlite>,
    dimension: &str,
    series_key: Option<&str>,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<LongTermRollupRow>> {
    let mut sql = format!(
        "{} SELECT stats_date AS bucket_or_date, dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples FROM active_daily WHERE dimension = ?1 AND stats_date BETWEEN ?2 AND ?3",
        long_term_projection_active_daily_cte(),
    );
    if series_key.is_some() {
        sql.push_str(" AND series_key = ?4");
    }
    sql.push_str(" ORDER BY stats_date ASC, series_key ASC");
    let mut query = sqlx::query_as::<_, LongTermRollupRow>(&sql)
        .bind(dimension)
        .bind(start_date)
        .bind(end_date);
    if let Some(series_key) = series_key {
        query = query.bind(series_key);
    }
    Ok(query.fetch_all(pool).await?)
}

fn long_term_projection_active_daily_cte() -> &'static str {
    r#"
    WITH active_daily AS (
      SELECT d.stats_date, d.dimension, d.series_key, d.display_name, d.reasoning_effort,
        d.calls, d.token_total, d.token_samples, d.cost_total, d.cost_samples,
        d.usage_time_ms, d.usage_time_samples, d.wall_time_ms, d.wall_time_samples,
        d.output_tokens_total, d.stream_duration_ms, d.output_speed_samples,
        d.first_byte_sum_ms, d.first_byte_samples, d.response_sum_ms, d.response_samples
      FROM long_term_usage_daily d
      LEFT JOIN long_term_projection_bucket_state state ON state.bucket_date = d.stats_date
      LEFT JOIN long_term_projection_date_publications publication
        ON publication.publication_token = state.publication_token
      WHERE state.active_daily_backup_token IS NULL
         OR (
           publication.published = 1
           AND NOT EXISTS (
             SELECT 1
             FROM long_term_projection_dirty_buckets dirty
             WHERE dirty.bucket_date = d.stats_date
               AND (
                 state.publication_generation IS NULL
                 OR dirty.generation <> state.publication_generation
               )
           )
         )
      UNION ALL
      SELECT backup.stats_date, backup.dimension, backup.series_key, backup.display_name,
        backup.reasoning_effort, backup.calls, backup.token_total, backup.token_samples,
        backup.cost_total, backup.cost_samples, backup.usage_time_ms,
        backup.usage_time_samples, backup.wall_time_ms, backup.wall_time_samples,
        backup.output_tokens_total, backup.stream_duration_ms, backup.output_speed_samples,
        backup.first_byte_sum_ms, backup.first_byte_samples, backup.response_sum_ms,
        backup.response_samples
      FROM long_term_projection_daily_backups backup
      JOIN long_term_projection_bucket_state state
        ON state.bucket_date = backup.stats_date
       AND state.active_daily_backup_token = backup.rebuild_token
      LEFT JOIN long_term_projection_date_publications publication
        ON publication.publication_token = state.publication_token
      WHERE publication.published IS NULL
         OR publication.published = 0
         OR EXISTS (
           SELECT 1
           FROM long_term_projection_dirty_buckets dirty
           WHERE dirty.bucket_date = backup.stats_date
             AND (
               state.publication_generation IS NULL
               OR dirty.generation <> state.publication_generation
             )
         )
    )
    "#
}

pub(crate) async fn fetch_long_term_overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LongTermRangeQuery>,
) -> Result<Json<LongTermStatsOverviewResponse>, (StatusCode, String)> {
    let range = LongTermRange::parse(query.range.as_deref())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "invalid range".to_string()))?;
    let state_row = load_long_term_state(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let status = normalize_long_term_response_status(&state_row.status);
    if status != LONG_TERM_STATUS_READY {
        return Ok(Json(LongTermStatsOverviewResponse {
            status,
            statistics_start_date: state_row.statistics_start_date,
            processed_rows: state_row.processed_rows,
            total_rows: state_row.total_rows,
            timezone: LONG_TERM_TIMEZONE,
            range: range.as_str().to_string(),
            global: LongTermMetrics::default(),
            daily: Vec::new(),
            models: Vec::new(),
            upstreams: Vec::new(),
        }));
    }
    let (start_date, end_date) =
        long_term_date_window(range, state_row.statistics_start_date.as_deref());
    let daily_rows =
        load_long_term_daily_rows(&state.pool, "overall", None, &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?;
    let global = aggregate_rollup_rows(&daily_rows);
    let daily = build_daily_points(&daily_rows, &start_date, &end_date);
    let models = build_series_summaries(
        &load_long_term_rollups(&state.pool, range, "model", &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?,
    );
    let upstreams = build_series_summaries(
        &load_long_term_rollups(&state.pool, range, "upstream", &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?,
    );
    Ok(Json(LongTermStatsOverviewResponse {
        status,
        statistics_start_date: state_row.statistics_start_date,
        processed_rows: state_row.processed_rows,
        total_rows: state_row.total_rows,
        timezone: LONG_TERM_TIMEZONE,
        range: range.as_str().to_string(),
        global,
        daily,
        models,
        upstreams,
    }))
}

pub(crate) async fn fetch_long_term_series(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
) -> Result<Json<LongTermStatsSeriesResponse>, (StatusCode, String)> {
    let query = parse_long_term_series_query(&original_uri);
    let range = LongTermRange::parse(query.range.as_deref())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "invalid range".to_string()))?;
    let dimension = query.dimension.as_deref().unwrap_or_default();
    if !matches!(dimension, "model" | "upstream") {
        return Err((StatusCode::BAD_REQUEST, "invalid dimension".to_string()));
    }
    let keys = query
        .key
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if keys.is_empty() || keys.len() > 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "key must contain 1 to 8 series keys".to_string(),
        ));
    }
    let state_row = load_long_term_state(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let status = normalize_long_term_response_status(&state_row.status);
    if status != LONG_TERM_STATUS_READY {
        return Ok(Json(LongTermStatsSeriesResponse {
            status,
            statistics_start_date: state_row.statistics_start_date,
            processed_rows: state_row.processed_rows,
            total_rows: state_row.total_rows,
            timezone: LONG_TERM_TIMEZONE,
            range: range.as_str().to_string(),
            dimension: dimension.to_string(),
            series: Vec::new(),
        }));
    }
    let (start_date, end_date) =
        long_term_date_window(range, state_row.statistics_start_date.as_deref());
    let available = load_long_term_rollups(&state.pool, range, dimension, &start_date, &end_date)
        .await
        .map_err(internal_error_tuple)?;
    let available_keys = available
        .iter()
        .map(|row| row.series_key.as_str())
        .collect::<HashSet<_>>();
    if keys
        .iter()
        .any(|key| !available_keys.contains(key.as_str()))
    {
        return Err((StatusCode::BAD_REQUEST, "unknown series key".to_string()));
    }
    let mut series = Vec::with_capacity(keys.len());
    for key in keys {
        let matching =
            load_long_term_daily_rows(&state.pool, dimension, Some(&key), &start_date, &end_date)
                .await
                .map_err(internal_error_tuple)?;
        let summary = available
            .iter()
            .find(|row| row.series_key.as_str() == key.as_str());
        series.push(LongTermSeries {
            series_key: key,
            display_name: summary
                .map(|row| row.display_name.clone())
                .unwrap_or_default(),
            reasoning_effort: summary.and_then(|row| {
                (!row.reasoning_effort.is_empty()).then_some(row.reasoning_effort.clone())
            }),
            points: build_daily_points(&matching, &start_date, &end_date),
        });
    }
    Ok(Json(LongTermStatsSeriesResponse {
        status,
        statistics_start_date: state_row.statistics_start_date,
        processed_rows: state_row.processed_rows,
        total_rows: state_row.total_rows,
        timezone: LONG_TERM_TIMEZONE,
        range: range.as_str().to_string(),
        dimension: dimension.to_string(),
        series,
    }))
}

async fn load_long_term_state(pool: &Pool<Sqlite>) -> Result<LongTermStateRow> {
    Ok(sqlx::query_as::<_, LongTermStateRow>(
        "SELECT status, statistics_start_date, integrity_source_start_date, processed_rows, total_rows, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(pool)
    .await?)
}

fn normalize_long_term_response_status(status: &str) -> String {
    match status {
        LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY | LONG_TERM_STATUS_PREPARING => {
            status.to_string()
        }
        LONG_TERM_STATUS_DISABLED => LONG_TERM_STATUS_PREPARING.to_string(),
        LONG_TERM_STATUS_RUNNING => LONG_TERM_STATUS_PREPARING.to_string(),
        _ => LONG_TERM_STATUS_ERROR.to_string(),
    }
}

fn long_term_date_window(range: LongTermRange, start_date: Option<&str>) -> (String, String) {
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let requested_start = today - ChronoDuration::days(range.days() - 1);
    let effective_start = start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .map(|value| value.max(requested_start))
        .unwrap_or(requested_start);
    (effective_start.to_string(), today.to_string())
}

fn build_series_summaries(rows: &[LongTermRollupRow]) -> Vec<LongTermSeriesSummary> {
    let mut summaries = rows
        .iter()
        .map(|row| LongTermSeriesSummary {
            series_key: row.series_key.clone(),
            display_name: row.display_name.clone(),
            reasoning_effort: (!row.reasoning_effort.is_empty())
                .then_some(row.reasoning_effort.clone()),
            metrics: LongTermMetrics::from_rollup(row),
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .metrics
            .tokens
            .unwrap_or_default()
            .cmp(&left.metrics.tokens.unwrap_or_default())
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    summaries
}

fn aggregate_rollup_rows(rows: &[LongTermRollupRow]) -> LongTermMetrics {
    let mut acc = LongTermAccumulator::default();
    let mut wall_time_ms = 0.0;
    let mut wall_samples = 0_i64;
    for row in rows {
        acc.calls += row.calls;
        acc.token_total += row.token_total;
        acc.token_samples += row.token_samples;
        acc.cost_total += row.cost_total;
        acc.cost_samples += row.cost_samples;
        acc.usage_time_ms += row.usage_time_ms;
        acc.usage_time_samples += row.usage_time_samples;
        acc.output_tokens_total += row.output_tokens_total;
        acc.stream_duration_ms += row.stream_duration_ms;
        acc.output_speed_samples += row.output_speed_samples;
        acc.first_byte_sum_ms += row.first_byte_sum_ms;
        acc.first_byte_samples += row.first_byte_samples;
        acc.response_sum_ms += row.response_sum_ms;
        acc.response_samples += row.response_samples;
        wall_time_ms += row.wall_time_ms;
        wall_samples += row.wall_time_samples;
    }
    let mut metrics = LongTermMetrics::from_accumulator(&acc);
    metrics.wall_time_ms = (wall_samples > 0).then_some(wall_time_ms);
    metrics.wall_time_samples = wall_samples;
    metrics
}
