async fn persist_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    file_path: &str,
    archive_sha256: &str,
    file_fingerprint: &str,
    compatibility: LongTermArchiveCompatibility,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "INSERT INTO long_term_projection_archive_compatibility (file_path, archive_sha256, file_fingerprint, has_legacy_crossing, legacy_max_duration_ms, legacy_min_occurred_at, has_rfc3339, rfc3339_max_duration_ms, rfc3339_min_occurred_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(file_path) DO UPDATE SET archive_sha256 = excluded.archive_sha256, file_fingerprint = excluded.file_fingerprint, has_legacy_crossing = excluded.has_legacy_crossing, legacy_max_duration_ms = excluded.legacy_max_duration_ms, legacy_min_occurred_at = excluded.legacy_min_occurred_at, has_rfc3339 = excluded.has_rfc3339, rfc3339_max_duration_ms = excluded.rfc3339_max_duration_ms, rfc3339_min_occurred_at = excluded.rfc3339_min_occurred_at, updated_at = datetime('now')",
    )
    .bind(file_path)
    .bind(archive_sha256)
    .bind(file_fingerprint)
    .bind(compatibility.has_legacy_crossing)
    .bind(compatibility.legacy_max_duration_ms)
    .bind(&compatibility.legacy_min_occurred_at)
    .bind(compatibility.has_rfc3339)
    .bind(compatibility.rfc3339_max_duration_ms)
    .bind(&compatibility.rfc3339_min_occurred_at)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}

async fn load_or_inspect_long_term_archive_compatibility(
    pool: &Pool<Sqlite>,
    archive_pool: &Pool<Sqlite>,
    file_path: &str,
    file_fingerprint: &str,
    query: &LongTermArchiveInvocationQueryParts,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermArchiveCompatibility> {
    let archive_sha256 = load_long_term_archive_sha256(pool, file_path).await?;
    if let Some(archive_sha256) = archive_sha256.as_deref()
        && let Some(compatibility) =
            load_long_term_archive_compatibility(pool, file_path, archive_sha256, file_fingerprint)
                .await?
    {
        return Ok(compatibility);
    }

    // Old archives can contain legacy or RFC3339 timestamps. Inspect once, then retain the
    // opened archive's checksum-and-file identity so ordinary canonical repairs stay range-seeked.
    let compatibility =
        inspect_long_term_archive_compatibility(archive_pool, query, control).await?;
    if let Some(archive_sha256) = archive_sha256.as_deref() {
        persist_long_term_archive_compatibility(
            pool,
            file_path,
            archive_sha256,
            file_fingerprint,
            compatibility.clone(),
            control,
        )
        .await?;
    }
    Ok(compatibility)
}

fn long_term_archive_file_fingerprint(file_path: &str) -> Result<String> {
    // Archive manifests are updated separately from file replacement. Hash the opened source
    // bytes rather than its mutable metadata so a stale manifest cannot reuse old capability.
    let mut file = std::fs::File::open(file_path).with_context(|| {
        format!("failed to open long-term archive {file_path} for fingerprinting")
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes_read = std::io::Read::read(&mut file, &mut buffer)
            .with_context(|| format!("failed to fingerprint long-term archive {file_path}"))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn long_term_archive_scan_identity_matches_manifest(
    scanned_sha256: &str,
    current_file_sha256: Option<&str>,
    manifest_sha256: Option<&str>,
) -> bool {
    current_file_sha256 == Some(scanned_sha256) && manifest_sha256 == Some(scanned_sha256)
}

fn long_term_archive_file_identity(file_path: &str) -> Result<String> {
    let metadata = std::fs::metadata(file_path)
        .with_context(|| format!("failed to stat long-term archive {file_path}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!(
            "unix:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        let modified_at = metadata
            .modified()
            .with_context(|| format!("failed to read long-term archive timestamp {file_path}"))?
            .duration_since(std::time::UNIX_EPOCH)
            .with_context(|| {
                format!("long-term archive timestamp predates the epoch {file_path}")
            })?;
        Ok(format!(
            "portable:{}:{}:{}",
            metadata.len(),
            modified_at.as_secs(),
            modified_at.subsec_nanos()
        ))
    }
}

async fn ensure_long_term_archive_source_identity(
    pool: &Pool<Sqlite>,
    dataset: &str,
    file_path: &str,
    expected_sha256: &str,
) -> Result<()> {
    let file_sha256 = crate::maintenance::sha256_hex_file(std::path::Path::new(file_path))?;
    let manifest_sha256 =
        load_long_term_archive_sha256_for_dataset(pool, dataset, file_path).await?;
    if long_term_archive_scan_identity_matches_manifest(
        expected_sha256,
        Some(&file_sha256),
        manifest_sha256.as_deref(),
    ) {
        Ok(())
    } else {
        bail!(
            "long-term archive source identity does not match its completed manifest: {file_path}"
        )
    }
}

async fn long_term_archive_pool_fingerprint(pool: &Pool<Sqlite>) -> Result<String> {
    let databases = sqlx::query_as::<_, (i64, String, String)>("PRAGMA database_list")
        .fetch_all(pool)
        .await
        .context("failed to resolve opened long-term archive database")?;
    let file_path = databases
        .into_iter()
        .find_map(|(_, name, file_path)| (name == "main").then_some(file_path))
        .filter(|file_path| !file_path.is_empty())
        .context("opened long-term archive does not expose a main database path")?;
    long_term_archive_file_fingerprint(&file_path)
}

fn long_term_archive_legacy_crossing_start(
    start: &chrono::DateTime<chrono_tz::Tz>,
    max_duration_ms: f64,
) -> Option<String> {
    if !max_duration_ms.is_finite() || max_duration_ms <= 0.0 {
        return None;
    }
    let max_duration_ms = max_duration_ms.ceil();
    if max_duration_ms > (i64::MAX - 1_000) as f64 {
        return None;
    }
    start
        .checked_sub_signed(ChronoDuration::milliseconds(max_duration_ms as i64 + 1_000))
        .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
}

fn long_term_rfc3339_text_bounds(
    start: chrono::DateTime<chrono_tz::Tz>,
    end: chrono::DateTime<chrono_tz::Tz>,
    compatibility: &LongTermRfc3339Compatibility,
) -> (String, String) {
    // RFC3339 input is accepted through -14:00 to +14:00. Relative to the +08:00 reporting
    // zone, the raw text for an instant can therefore be twenty-two hours earlier than its local
    // reporting time; leave one extra second for the exclusive lower boundary.
    const LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS: i64 = 22 * 60 * 60 + 1;
    let lower = compatibility
        .max_duration_ms
        .filter(|duration_ms| duration_ms.is_finite() && *duration_ms > 0.0)
        .and_then(|duration_ms| {
            let seconds = (duration_ms / 1000.0).ceil();
            (seconds <= (i64::MAX - LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS) as f64)
                .then_some(seconds as i64)
        })
        .and_then(|seconds| {
            start.checked_sub_signed(ChronoDuration::seconds(
                seconds + LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS,
            ))
        })
        .map(|value| value.format("%Y-%m-%dT%H:%M:%S").to_string())
        // Zero-duration rows cannot cross into the target date. Keep their candidate seek to the
        // RFC3339 offset window instead of reopening the range at the archive's first old row.
        .unwrap_or_else(|| {
            start
                .checked_sub_signed(ChronoDuration::seconds(
                    LONG_TERM_RFC3339_TEXT_LOWER_OFFSET_SECONDS,
                ))
                .unwrap_or(start)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        });
    let upper = end
        .checked_add_signed(ChronoDuration::hours(15))
        .unwrap_or(end)
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    (lower, upper)
}

async fn load_long_term_archive_invocation_rows_for_range(
    pool: &Pool<Sqlite>,
    queries: &LongTermArchiveInvocationRangeQueries,
    compatibility: LongTermArchiveCompatibility,
    start: chrono::DateTime<chrono_tz::Tz>,
    end: chrono::DateTime<chrono_tz::Tz>,
) -> Result<Vec<LongTermInvocationRow>> {
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
    let mut rows = sqlx::query_as::<_, LongTermInvocationRow>(&queries.canonical)
        .bind(&start_text)
        .bind(&end_text)
        .fetch_all(pool)
        .await?;
    if compatibility.has_legacy_crossing {
        let crossing_start = compatibility
            .legacy_max_duration_ms
            .and_then(|max_duration_ms| {
                long_term_archive_legacy_crossing_start(&start, max_duration_ms)
            })
            .or_else(|| compatibility.legacy_min_occurred_at.clone())
            .context("long-term archive legacy compatibility cache is missing a bounded start")?;
        let crossing_rows = sqlx::query_as::<_, LongTermInvocationRow>(&queries.crossing_text)
            .bind(crossing_start)
            .bind(&start_text)
            .fetch_all(pool)
            .await?;
        rows.extend(crossing_rows);
    }
    if compatibility.has_rfc3339 {
        let rfc3339_compatibility = LongTermRfc3339Compatibility {
            max_duration_ms: compatibility.rfc3339_max_duration_ms,
        };
        let (rfc3339_lower, rfc3339_upper) =
            long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
        rows.extend(
            sqlx::query_as::<_, LongTermInvocationRow>(&queries.rfc3339)
                .bind(rfc3339_lower)
                .bind(rfc3339_upper)
                .bind(start.timestamp())
                .bind(end.timestamp())
                .fetch_all(pool)
                .await?,
        );
    }
    // The canonical, legacy-crossing, and RFC3339 queries each preserve their own seek order.
    // Their union has no single text order across timestamp encodings, so retain a stable archive
    // row order without reinterpreting timestamps after the sargable range seeks.
    rows.sort_by_key(|row| row.id);
    Ok(rows)
}

fn long_term_rfc3339_whole_second_sql(value: &str) -> String {
    // SQLite normalizes RFC3339 fractions to milliseconds before evaluating `strftime` or
    // `julianday`. Strip the fraction before finding the whole second, then retain its original
    // digits so both exact boundaries and sub-millisecond values preserve their true ordering.
    let fraction_tail = format!("substr({value}, 21)");
    let fraction_end = format!(
        "CASE WHEN instr({fraction_tail}, 'Z') > 0 THEN instr({fraction_tail}, 'Z') - 1 WHEN instr({fraction_tail}, '+') > 0 THEN instr({fraction_tail}, '+') - 1 WHEN instr({fraction_tail}, '-') > 0 THEN instr({fraction_tail}, '-') - 1 ELSE length({value}) - 20 END"
    );
    let whole_second = format!(
        "CASE WHEN substr({value}, 20, 1) = '.' THEN substr({value}, 1, 19) || substr({value}, 21 + ({fraction_end})) ELSE {value} END"
    );
    whole_second
}

fn long_term_rfc3339_fraction_nanos_sql(value: &str) -> String {
    let fraction_tail = format!("substr({value}, 21)");
    let fraction_end = format!(
        "CASE WHEN instr({fraction_tail}, 'Z') > 0 THEN instr({fraction_tail}, 'Z') - 1 WHEN instr({fraction_tail}, '+') > 0 THEN instr({fraction_tail}, '+') - 1 WHEN instr({fraction_tail}, '-') > 0 THEN instr({fraction_tail}, '-') - 1 ELSE length({value}) - 20 END"
    );
    format!(
        "CASE WHEN substr({value}, 20, 1) = '.' THEN CAST(substr(substr({value}, 21, {fraction_end}) || '000000000', 1, 9) AS INTEGER) ELSE 0 END"
    )
}

fn long_term_rfc3339_whole_epoch_seconds_sql(value: &str) -> String {
    format!(
        "CAST(strftime('%s', {}) AS INTEGER)",
        long_term_rfc3339_whole_second_sql(value)
    )
}

fn long_term_rfc3339_elapsed_nanos_sql(duration_ms: &str) -> String {
    // Timestamps retain the first nine fractional digits as nanoseconds. The elapsed value is a
    // persisted SQLite REAL, so normalize only that bounded duration before comparing it with the
    // exact timestamp digits; never turn the RFC3339 fraction itself into a REAL.
    format!("CAST(ROUND(MAX(COALESCE({duration_ms}, 0), 0) * 1000000.0) AS INTEGER)")
}

fn long_term_rfc3339_fractional_carry_sql(value: &str, duration_ms: &str) -> String {
    let fraction_nanos = long_term_rfc3339_fraction_nanos_sql(value);
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(duration_ms);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let elapsed_fraction_nanos =
        format!("(({elapsed_nanos}) - ({elapsed_whole_seconds}) * 1000000000)");
    format!(
        "CASE WHEN ({fraction_nanos}) + ({elapsed_fraction_nanos}) >= 1000000000 THEN 1 ELSE 0 END"
    )
}

fn long_term_rfc3339_reaches_epoch_sql(value: &str, duration_ms: &str, epoch: &str) -> String {
    let whole_epoch = long_term_rfc3339_whole_epoch_seconds_sql(value);
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(duration_ms);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let fractional_carry = long_term_rfc3339_fractional_carry_sql(value, duration_ms);
    format!("(({whole_epoch}) + ({elapsed_whole_seconds}) + ({fractional_carry}) >= {epoch})")
}

fn long_term_rfc3339_shanghai_date_sql(value: &str, duration_ms: Option<&str>) -> String {
    // `date(..., 'unixepoch')` normalizes fractional seconds to milliseconds. Keep the raw
    // fractional tail outside date arithmetic, and carry it into the next exact second only
    // when a positive duration crosses that second.
    let whole_epoch = long_term_rfc3339_whole_epoch_seconds_sql(value);
    let elapsed_seconds = duration_ms.unwrap_or("0");
    let elapsed_nanos = long_term_rfc3339_elapsed_nanos_sql(elapsed_seconds);
    let elapsed_whole_seconds = format!("CAST(({elapsed_nanos}) / 1000000000 AS INTEGER)");
    let fractional_carry = long_term_rfc3339_fractional_carry_sql(value, elapsed_seconds);
    format!(
        "date(({whole_epoch}) + ({elapsed_whole_seconds}) + ({fractional_carry}), 'unixepoch', '+8 hours')"
    )
}

async fn long_term_archive_invocation_query_parts(
    pool: &Pool<Sqlite>,
) -> Result<LongTermArchiveInvocationQueryParts> {
    let columns = load_archive_table_columns(pool, "codex_invocations").await?;
    let select = |column: &str| long_term_legacy_select_expr(&columns, column);
    let status_column = if columns.contains("status") {
        "status"
    } else {
        "NULL"
    };
    let payload = if columns.contains("payload") {
        "payload"
    } else {
        "NULL"
    };
    let request_model = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.requestModel') AS TEXT)), '') END"
    );
    let response_model = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.responseModel') AS TEXT)), '') END"
    );
    let reasoning_effort = format!(
        "CASE WHEN json_valid({payload}) THEN NULLIF(TRIM(CAST(json_extract({payload}, '$.reasoningEffort') AS TEXT)), '') END"
    );
    let payload_upstream_account_id = format!(
        "CASE WHEN json_valid({payload}) THEN CAST(json_extract({payload}, '$.upstreamAccountId') AS INTEGER) END"
    );
    let upstream_account_id = if columns.contains("upstream_account_id") {
        format!("COALESCE(upstream_account_id, {payload_upstream_account_id})")
    } else {
        payload_upstream_account_id
    };
    let t_total_ms_column = if columns.contains("t_total_ms") {
        "t_total_ms"
    } else {
        "NULL"
    };
    let select = format!(
        r#"
        SELECT
            id,
            {invoke_id},
            occurred_at,
            {status},
            {model},
            {request_model} AS request_model,
            {response_model} AS response_model,
            {reasoning_effort} AS reasoning_effort,
            {upstream_account_id} AS upstream_account_id,
            NULL AS upstream_account_kind,
            NULL AS upstream_account_name,
            {total_tokens},
            {output_tokens},
            {cost},
            {t_total_ms},
            {t_req_read_ms},
            {t_req_parse_ms},
            {t_upstream_connect_ms},
            {t_upstream_ttfb_ms},
            {t_upstream_stream_ms},
            {error_message}
        FROM codex_invocations
        "#,
        invoke_id = select("invoke_id"),
        status = select("status"),
        model = select("model"),
        request_model = request_model,
        response_model = response_model,
        reasoning_effort = reasoning_effort,
        upstream_account_id = upstream_account_id,
        total_tokens = select("total_tokens"),
        output_tokens = select("output_tokens"),
        cost = select("cost"),
        t_total_ms = select("t_total_ms"),
        t_req_read_ms = select("t_req_read_ms"),
        t_req_parse_ms = select("t_req_parse_ms"),
        t_upstream_connect_ms = select("t_upstream_connect_ms"),
        t_upstream_ttfb_ms = select("t_upstream_ttfb_ms"),
        t_upstream_stream_ms = select("t_upstream_stream_ms"),
        error_message = select("error_message"),
    );
    Ok(LongTermArchiveInvocationQueryParts {
        select,
        terminal_filter: format!(
            "LOWER(TRIM(COALESCE({status_column}, ''))) NOT IN ('running', 'pending')"
        ),
        status_column: status_column.to_string(),
        t_total_ms_column: t_total_ms_column.to_string(),
    })
}

fn long_term_legacy_select_expr(columns: &HashSet<String>, column: &str) -> String {
    format!(
        "{} AS {column}",
        long_term_legacy_column_expr(columns, column)
    )
}

fn long_term_legacy_column_expr(columns: &HashSet<String>, column: &str) -> String {
    if columns.contains(column) {
        column.to_string()
    } else {
        "NULL".to_string()
    }
}

#[derive(Debug, Clone, FromRow)]
struct LongTermRollupRow {
    bucket_or_date: String,
    dimension: String,
    series_key: String,
    display_name: String,
    reasoning_effort: String,
    calls: i64,
    token_total: i64,
    token_samples: i64,
    cost_total: f64,
    cost_samples: i64,
    usage_time_ms: f64,
    usage_time_samples: i64,
    wall_time_ms: f64,
    wall_time_samples: i64,
    output_tokens_total: i64,
    stream_duration_ms: f64,
    output_speed_samples: i64,
    first_byte_sum_ms: f64,
    first_byte_samples: i64,
    response_sum_ms: f64,
    response_samples: i64,
}

#[derive(Debug, Default, Clone)]
struct LongTermAccumulator {
    calls: i64,
    token_total: i64,
    token_samples: i64,
    cost_total: f64,
    cost_samples: i64,
    usage_time_ms: f64,
    usage_time_samples: i64,
    output_tokens_total: i64,
    stream_duration_ms: f64,
    output_speed_samples: i64,
    first_byte_sum_ms: f64,
    first_byte_samples: i64,
    response_sum_ms: f64,
    response_samples: i64,
    intervals: Vec<(i64, i64)>,
}

#[derive(Debug, Clone)]
struct LongTermBucket {
    bucket_start_epoch: i64,
    dimension: String,
    series_key: String,
    display_name: String,
    reasoning_effort: String,
    stats_date: Option<String>,
    accumulator: LongTermAccumulator,
}

impl LongTermAccumulator {
    fn add_call(&mut self, row: &LongTermInvocationRow, interval: Option<(i64, i64)>) {
        self.calls += 1;
        if let Some(tokens) = row.total_tokens {
            self.token_total += tokens.max(0);
            self.token_samples += 1;
        }
        if let Some(cost) = row.cost.filter(|value| value.is_finite()) {
            self.cost_total += cost;
            self.cost_samples += 1;
        }
        if !is_success_status(row.status.as_deref(), row.error_message.as_deref()) {
            return;
        }
        if let Some(value) = row
            .t_total_ms
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            self.usage_time_ms += value;
            self.usage_time_samples += 1;
        }
        if let (Some(output_tokens), Some(stream_duration_ms)) = (
            row.output_tokens,
            row.t_upstream_stream_ms
                .filter(|value| value.is_finite() && *value > 0.0),
        ) {
            self.output_tokens_total += output_tokens.max(0);
            self.stream_duration_ms += stream_duration_ms;
            self.output_speed_samples += 1;
        }
        if let Some(value) = crate::stats::resolve_first_response_byte_total_ms(
            row.t_req_read_ms,
            row.t_req_parse_ms,
            row.t_upstream_connect_ms,
            row.t_upstream_ttfb_ms,
        ) {
            self.first_byte_sum_ms += value;
            self.first_byte_samples += 1;
        }
        if let Some(value) = row
            .t_upstream_stream_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.response_sum_ms += value;
            self.response_samples += 1;
        }
        if let Some((start, end)) = interval {
            self.intervals.push((start, end));
        }
    }

    fn merge(&mut self, other: &Self) {
        self.calls += other.calls;
        self.token_total += other.token_total;
        self.token_samples += other.token_samples;
        self.cost_total += other.cost_total;
        self.cost_samples += other.cost_samples;
        self.usage_time_ms += other.usage_time_ms;
        self.usage_time_samples += other.usage_time_samples;
        self.output_tokens_total += other.output_tokens_total;
        self.stream_duration_ms += other.stream_duration_ms;
        self.output_speed_samples += other.output_speed_samples;
        self.first_byte_sum_ms += other.first_byte_sum_ms;
        self.first_byte_samples += other.first_byte_samples;
        self.response_sum_ms += other.response_sum_ms;
        self.response_samples += other.response_samples;
        self.intervals.extend_from_slice(&other.intervals);
    }

    fn wall_time_ms(&self) -> f64 {
        union_interval_duration(&self.intervals) as f64
    }

    fn wall_sample_count(&self) -> i64 {
        self.intervals.len() as i64
    }

    fn add_interval(&mut self, interval: Option<(i64, i64)>) {
        if let Some(interval) = interval {
            self.intervals.push(interval);
        }
    }
}

impl LongTermMetrics {
    fn from_accumulator(acc: &LongTermAccumulator) -> Self {
        Self {
            calls: acc.calls,
            tokens: (acc.token_samples > 0).then_some(acc.token_total),
            token_samples: acc.token_samples,
            cost: (acc.cost_samples > 0).then_some(acc.cost_total),
            cost_samples: acc.cost_samples,
            usage_time_ms: (acc.usage_time_samples > 0).then_some(acc.usage_time_ms),
            usage_time_samples: acc.usage_time_samples,
            wall_time_ms: (!acc.intervals.is_empty()).then_some(acc.wall_time_ms()),
            wall_time_samples: acc.wall_sample_count(),
            output_speed_tokens_per_second: (acc.output_speed_samples > 0
                && acc.stream_duration_ms > 0.0)
                .then_some(acc.output_tokens_total as f64 / (acc.stream_duration_ms / 1000.0)),
            output_speed_samples: acc.output_speed_samples,
            first_byte_ms: (acc.first_byte_samples > 0)
                .then_some(acc.first_byte_sum_ms / acc.first_byte_samples as f64),
            first_byte_samples: acc.first_byte_samples,
            response_ms: (acc.response_samples > 0)
                .then_some(acc.response_sum_ms / acc.response_samples as f64),
            response_samples: acc.response_samples,
        }
    }

    fn from_rollup(row: &LongTermRollupRow) -> Self {
        Self {
            calls: row.calls,
            tokens: (row.token_samples > 0).then_some(row.token_total),
            token_samples: row.token_samples,
            cost: (row.cost_samples > 0).then_some(row.cost_total),
            cost_samples: row.cost_samples,
            usage_time_ms: (row.usage_time_samples > 0).then_some(row.usage_time_ms),
            usage_time_samples: row.usage_time_samples,
            wall_time_ms: (row.wall_time_samples > 0).then_some(row.wall_time_ms),
            wall_time_samples: row.wall_time_samples,
            output_speed_tokens_per_second: (row.output_speed_samples > 0
                && row.stream_duration_ms > 0.0)
                .then_some(row.output_tokens_total as f64 / (row.stream_duration_ms / 1000.0)),
            output_speed_samples: row.output_speed_samples,
            first_byte_ms: (row.first_byte_samples > 0)
                .then_some(row.first_byte_sum_ms / row.first_byte_samples as f64),
            first_byte_samples: row.first_byte_samples,
            response_ms: (row.response_samples > 0)
                .then_some(row.response_sum_ms / row.response_samples as f64),
            response_samples: row.response_samples,
        }
    }
}

pub(crate) fn ensure_long_term_stats_schema_sql() -> &'static str {
    "long_term_usage_hourly, long_term_usage_daily, long_term_stats_state and long_term_stats_repair_queue"
}

pub(crate) async fn ensure_long_term_stats_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_long_term_usage_rollup_tables(pool).await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_stats_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            status TEXT NOT NULL DEFAULT 'preparing',
            statistics_start_date TEXT,
            integrity_source_start_date TEXT,
            integrity_source_pending_start_date TEXT,
            processed_rows INTEGER NOT NULL DEFAULT 0,
            total_rows INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_stats_state table")?;
    sqlx::query("INSERT OR IGNORE INTO long_term_stats_state (id, status) VALUES (?1, ?2)")
        .bind(LONG_TERM_STATE_ID)
        .bind(LONG_TERM_STATUS_PREPARING)
        .execute(pool)
        .await
        .context("failed to seed long term stats state")?;
    let state_columns = load_sqlite_table_columns(pool, "long_term_stats_state").await?;
    if !state_columns.contains("last_integrity_audit_at") {
        sqlx::query("ALTER TABLE long_term_stats_state ADD COLUMN last_integrity_audit_at TEXT")
            .execute(pool)
            .await
            .context("failed to add long term integrity audit timestamp")?;
    }
    if !state_columns.contains("integrity_source_start_date") {
        sqlx::query(
            "ALTER TABLE long_term_stats_state ADD COLUMN integrity_source_start_date TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add long term integrity source boundary")?;
    }
    if !state_columns.contains("integrity_source_pending_start_date") {
        sqlx::query(
            "ALTER TABLE long_term_stats_state ADD COLUMN integrity_source_pending_start_date TEXT",
        )
        .execute(pool)
        .await
        .context("failed to add pending long term integrity source boundary")?;
    }
    ensure_long_term_repair_queue_schema(pool).await?;
    ensure_long_term_archive_replay_schema(pool).await?;
    ensure_long_term_projection_schema(pool).await?;
    ensure_long_term_projection_source_indexes(pool).await?;
    ensure_long_term_projection_correction_trigger(pool).await?;
    ensure_long_term_projection_archive_trigger(pool).await?;
    Ok(())
}

async fn ensure_long_term_usage_rollup_tables(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_usage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            reasoning_effort TEXT NOT NULL DEFAULT '',
            calls INTEGER NOT NULL DEFAULT 0,
            token_total INTEGER NOT NULL DEFAULT 0,
            token_samples INTEGER NOT NULL DEFAULT 0,
            cost_total REAL NOT NULL DEFAULT 0,
            cost_samples INTEGER NOT NULL DEFAULT 0,
            usage_time_ms REAL NOT NULL DEFAULT 0,
            usage_time_samples INTEGER NOT NULL DEFAULT 0,
            wall_time_ms REAL NOT NULL DEFAULT 0,
            wall_time_samples INTEGER NOT NULL DEFAULT 0,
            output_tokens_total INTEGER NOT NULL DEFAULT 0,
            stream_duration_ms REAL NOT NULL DEFAULT 0,
            output_speed_samples INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_samples INTEGER NOT NULL DEFAULT 0,
            response_sum_ms REAL NOT NULL DEFAULT 0,
            response_samples INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, dimension, series_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_usage_hourly table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_usage_daily (
            stats_date TEXT NOT NULL,
            dimension TEXT NOT NULL,
            series_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            reasoning_effort TEXT NOT NULL DEFAULT '',
            calls INTEGER NOT NULL DEFAULT 0,
            token_total INTEGER NOT NULL DEFAULT 0,
            token_samples INTEGER NOT NULL DEFAULT 0,
            cost_total REAL NOT NULL DEFAULT 0,
            cost_samples INTEGER NOT NULL DEFAULT 0,
            usage_time_ms REAL NOT NULL DEFAULT 0,
            usage_time_samples INTEGER NOT NULL DEFAULT 0,
            wall_time_ms REAL NOT NULL DEFAULT 0,
            wall_time_samples INTEGER NOT NULL DEFAULT 0,
            output_tokens_total INTEGER NOT NULL DEFAULT 0,
            stream_duration_ms REAL NOT NULL DEFAULT 0,
            output_speed_samples INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_samples INTEGER NOT NULL DEFAULT 0,
            response_sum_ms REAL NOT NULL DEFAULT 0,
            response_samples INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (stats_date, dimension, series_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long_term_usage_daily table")?;
    Ok(())
}

async fn ensure_long_term_repair_queue_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS long_term_stats_repair_queue (
            stats_date TEXT PRIMARY KEY,
            expected_calls INTEGER NOT NULL,
            expected_token_total INTEGER NOT NULL,
            expected_cost_total REAL NOT NULL,
            observed_calls INTEGER NOT NULL,
            observed_token_total INTEGER NOT NULL,
            observed_cost_total REAL NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            next_retry_at TEXT NOT NULL DEFAULT (datetime('now')),
            last_error TEXT NOT NULL,
            detected_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long term integrity repair queue")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_stats_repair_queue_due ON long_term_stats_repair_queue (next_retry_at, stats_date)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term integrity repair queue index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_usage_daily_dimension_date ON long_term_usage_daily (dimension, stats_date)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term daily index")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_long_term_usage_hourly_dimension_bucket ON long_term_usage_hourly (dimension, bucket_start_epoch)",
    )
    .execute(pool)
    .await
    .context("failed to ensure long term hourly index")?;
    Ok(())
}

async fn ensure_long_term_archive_replay_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            archive_sha256 TEXT,
            source_identity TEXT,
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure long term archive replay marker table")?;
    let replay_columns = load_sqlite_table_columns(pool, "hourly_rollup_archive_replay").await?;
    if !replay_columns.contains("archive_sha256") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN archive_sha256 TEXT")
            .execute(pool)
            .await
            .context("failed to add archive hash to replay markers")?;
    }
    if !replay_columns.contains("source_identity") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN source_identity TEXT")
            .execute(pool)
            .await
            .context("failed to add archive file identity to replay markers")?;
    }
    Ok(())
}

const LONG_TERM_PROJECTION_CONSUMER: &str = "long_term_v1";
const LONG_TERM_PROJECTION_FLUSH_INTERVAL: Duration = Duration::from_secs(60);
const LONG_TERM_PROJECTION_REPAIR_INTERVAL: Duration = Duration::from_secs(5 * 60);
const LONG_TERM_PROJECTION_DAILY_VERIFY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const LONG_TERM_PROJECTION_MAX_BUCKETS_PER_FLUSH: i64 = 1;
const LONG_TERM_PROJECTION_MAX_EVENTS_PER_FLUSH: i64 = 2_000;
const LONG_TERM_PROJECTION_WRITE_BATCH_ROWS: usize = 512;
const LONG_TERM_PROJECTION_ADMISSION_WAIT: Duration = Duration::from_millis(250);
const LONG_TERM_PROJECTION_TRANSACTION_WAIT: Duration = Duration::from_millis(250);
// An incremental publication persists canonical interval state, rollups, its cursor, and status
// atomically. Keep every mutation in that one short transaction below the shared write limit.
const LONG_TERM_PROJECTION_INCREMENTAL_METADATA_ROWS: usize = 2;
const LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS: usize =
    LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - LONG_TERM_PROJECTION_INCREMENTAL_METADATA_ROWS;
// A rebuild segment updates canonical state, membership, and a suppression row. Its first
// transaction also updates the bucket marker, so leave one row of headroom.
const LONG_TERM_PROJECTION_REBUILD_SEGMENT_ROWS: usize =
    (LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - 1) / 3;
// Staging more dates than this would make retry bookkeeping needlessly large. Publication is a
// single token transaction; each already-published date is released separately afterward.
const LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES: usize =
    (LONG_TERM_PROJECTION_WRITE_BATCH_ROWS - 2) / 3;
