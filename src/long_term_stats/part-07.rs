async fn load_long_term_source_timing_rows_for_pairs(
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

fn long_term_source_timing_archive_query(columns: &HashSet<String>) -> String {
    format!(
        "SELECT {} AS invoke_id, occurred_at, {} AS t_total_ms FROM codex_invocations",
        long_term_legacy_column_expr(columns, "invoke_id"),
        long_term_legacy_column_expr(columns, "t_total_ms"),
    )
}

fn long_term_archive_may_contain_attempt_pairs(
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

fn record_long_term_matched_attempt_source_rows(
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

fn long_term_source_effective_date(
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

fn long_term_integrity_totals_match(
    expected: LongTermIntegrityTotals,
    observed: LongTermIntegrityTotals,
) -> bool {
    expected.calls == observed.calls
        && expected.token_total == observed.token_total
        && (expected.cost_total - observed.cost_total).abs()
            <= 1e-6_f64.max(expected.cost_total.abs() * 1e-9)
}

fn long_term_integrity_totals_are_empty(totals: LongTermIntegrityTotals) -> bool {
    totals.calls == 0 && totals.token_total == 0 && totals.cost_total.abs() <= 1e-12
}

fn long_term_integrity_mismatch(
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

async fn long_term_integrity_oracle_available(pool: &Pool<Sqlite>) -> Result<bool> {
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

async fn load_long_term_integrity_oracle(
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

fn long_term_candidate_integrity(
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

fn remove_long_term_candidate_dates(
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

async fn enqueue_long_term_integrity_mismatch(
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

async fn load_long_term_reconciliation_mismatches(
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

async fn next_due_long_term_repair_date(
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

async fn queued_long_term_repair_mismatch(
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

async fn long_term_integrity_audit_due(pool: &Pool<Sqlite>) -> Result<bool> {
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

async fn mark_long_term_integrity_audit(
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

async fn audit_long_term_integrity(
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

fn long_term_repair_backoff_secs(attempts: i64) -> i64 {
    let index = attempts.saturating_sub(1) as usize;
    LONG_TERM_REPAIR_BACKOFF_SECS[index.min(LONG_TERM_REPAIR_BACKOFF_SECS.len() - 1)]
}

async fn schedule_long_term_repair_retry(
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

async fn mark_long_term_stats_backfill_preparing(pool: &Pool<Sqlite>) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    mark_long_term_stats_backfill_preparing_with_control(pool, &control).await
}

async fn mark_long_term_stats_backfill_preparing_with_control(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    // Preserve error across restart so a persisted integrity queue keeps the next refresh on the
    // verified incremental path. Error without durable rows still transitions to running inside
    // refresh_long_term_stats_once.
    let (mut transaction, permit) = control.begin(pool).await?;
    sqlx::query(
        "UPDATE long_term_stats_state SET status = ?1, updated_at = datetime('now') WHERE id = ?2 AND status NOT IN (?3, ?4)",
    )
    .bind(LONG_TERM_STATUS_PREPARING)
    .bind(LONG_TERM_STATE_ID)
    .bind(LONG_TERM_STATUS_READY)
    .bind(LONG_TERM_STATUS_ERROR)
    .execute(&mut *transaction)
    .await?;
    control.commit(transaction, permit).await
}
