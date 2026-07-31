use super::*;

const PARALLEL_WORK_ROLLUP_MAINTENANCE_HOUR_LIMIT: i64 = 24;
const PARALLEL_WORK_SCOPE_ALL: &str = "all";
const PARALLEL_WORK_SCOPE_PROXY_ONLY: &str = "proxy_only";

pub(crate) fn parallel_work_source_scope_name(source_scope: InvocationSourceScope) -> &'static str {
    match source_scope {
        InvocationSourceScope::All => PARALLEL_WORK_SCOPE_ALL,
        InvocationSourceScope::ProxyOnly => PARALLEL_WORK_SCOPE_PROXY_ONLY,
    }
}

fn current_hour_start_epoch(now: DateTime<Utc>) -> i64 {
    now.timestamp().div_euclid(3_600) * 3_600
}

async fn replace_parallel_work_minute_keys_for_hour_tx(
    tx: &mut SqliteConnection,
    hour_start_epoch: i64,
    full_detail_start_epoch: Option<i64>,
) -> Result<bool> {
    if full_detail_start_epoch.is_none_or(|cutoff| hour_start_epoch < cutoff) {
        return Ok(false);
    }
    let hour_end_epoch = hour_start_epoch + 3_600;
    let hour_start = Utc
        .timestamp_opt(hour_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid parallel-work hour start"))?;
    let hour_end = Utc
        .timestamp_opt(hour_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid parallel-work hour end"))?;
    let has_pruned_payloads: i64 = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM codex_invocations
            WHERE occurred_at >= ?1
                AND occurred_at < ?2
                AND detail_level != ?3
        )
        "#,
    )
    .bind(db_occurred_at_lower_bound(hour_start))
    .bind(db_occurred_at_lower_bound(hour_end))
    .bind(DETAIL_LEVEL_FULL)
    .fetch_one(&mut *tx)
    .await?;
    if has_pruned_payloads != 0 {
        return Ok(false);
    }
    for table in [
        "parallel_work_minute_key_rollup",
        "parallel_work_upstream_account_minute_key_rollup",
    ] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2"
        ))
        .bind(hour_start_epoch)
        .bind(hour_end_epoch)
        .execute(&mut *tx)
        .await?;
    }
    let rows =
        load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &[hour_start_epoch]).await?;
    upsert_parallel_work_minute_key_rollups_including_expired_tx(tx, &rows).await?;
    Ok(true)
}

pub(crate) async fn load_parallel_work_full_detail_start_epoch(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    sqlx::query_scalar(
        "SELECT full_detail_start_epoch FROM parallel_work_rollup_coverage_state WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn parallel_work_minute_coverage_ready_for_payload_retention(
    pool: &Pool<Sqlite>,
    payload_loss_cutoff_epoch: i64,
) -> Result<bool> {
    let keep_start_epoch = parallel_work_minute_rollup_keep_start_epoch(Utc::now())?;
    let coverage_start_epoch = load_parallel_work_full_detail_start_epoch(pool)
        .await?
        .unwrap_or(keep_start_epoch)
        .max(keep_start_epoch)
        .div_euclid(3_600)
        * 3_600;
    let coverage_end_epoch = payload_loss_cutoff_epoch.div_euclid(3_600) * 3_600;
    if coverage_end_epoch <= coverage_start_epoch {
        return Ok(true);
    }
    let expected_rows = ((coverage_end_epoch - coverage_start_epoch) / 3_600) * 2;
    let covered_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM parallel_work_hourly_coverage
        WHERE hour_start_epoch >= ?1
            AND hour_start_epoch < ?2
            AND source_scope IN (?3, ?4)
            AND minute_keys_complete = 1
        "#,
    )
    .bind(coverage_start_epoch)
    .bind(coverage_end_epoch)
    .bind(PARALLEL_WORK_SCOPE_ALL)
    .bind(PARALLEL_WORK_SCOPE_PROXY_ONLY)
    .fetch_one(pool)
    .await?;
    Ok(covered_rows == expected_rows)
}

async fn advance_parallel_work_full_detail_start_epoch(
    pool: &Pool<Sqlite>,
    current_full_detail_start_epoch: Option<i64>,
) -> Result<Option<i64>> {
    let Some(live_start_epoch) = current_full_detail_start_epoch else {
        return load_parallel_work_full_detail_start_epoch(pool).await;
    };
    let keep_start_epoch = parallel_work_minute_rollup_keep_start_epoch(Utc::now())?;
    let latest_unrecoverable_epoch: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT MAX(CASE
            WHEN instr(occurred_at, 'T') > 0
                THEN CAST(strftime('%s', occurred_at) AS INTEGER)
            ELSE CAST(strftime('%s', occurred_at || '+08:00') AS INTEGER)
        END)
        FROM codex_invocations
        WHERE (CASE
            WHEN instr(occurred_at, 'T') > 0
                THEN CAST(strftime('%s', occurred_at) AS INTEGER)
            ELSE CAST(strftime('%s', occurred_at || '+08:00') AS INTEGER)
        END) >= ?1
            AND detail_level != ?2
        "#,
    )
    .bind(keep_start_epoch)
    .bind(DETAIL_LEVEL_FULL)
    .fetch_one(pool)
    .await?;
    let verified_start_epoch = latest_unrecoverable_epoch
        .map(|epoch| (epoch.div_euclid(3_600) + 1) * 3_600)
        .unwrap_or(keep_start_epoch)
        .max(keep_start_epoch)
        .max(live_start_epoch.div_euclid(3_600) * 3_600);
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO parallel_work_rollup_coverage_state (id, full_detail_start_epoch)
        VALUES (1, ?1)
        ON CONFLICT(id) DO UPDATE SET
            full_detail_start_epoch = MIN(
                parallel_work_rollup_coverage_state.full_detail_start_epoch,
                excluded.full_detail_start_epoch
            )
        "#,
    )
    .bind(verified_start_epoch)
    .execute(tx.as_mut())
    .await?;
    let full_detail_start_epoch: i64 = sqlx::query_scalar(
        "SELECT full_detail_start_epoch FROM parallel_work_rollup_coverage_state WHERE id = 1",
    )
    .fetch_one(tx.as_mut())
    .await?;
    tx.commit().await?;
    Ok(Some(full_detail_start_epoch))
}

async fn mark_parallel_work_minute_coverage_tx(
    tx: &mut SqliteConnection,
    hour_start_epoch: i64,
) -> Result<()> {
    for source_scope in [PARALLEL_WORK_SCOPE_ALL, PARALLEL_WORK_SCOPE_PROXY_ONLY] {
        sqlx::query(
            r#"
            INSERT INTO parallel_work_hourly_coverage (
                hour_start_epoch, source_scope, minute_keys_complete, hourly_scalar_complete
            )
            VALUES (?1, ?2, 1, 0)
            ON CONFLICT(hour_start_epoch, source_scope) DO UPDATE SET
                minute_keys_complete = 1
            "#,
        )
        .bind(hour_start_epoch)
        .bind(source_scope)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn materialize_parallel_work_hour_tx(
    tx: &mut SqliteConnection,
    hour_start_epoch: i64,
) -> Result<()> {
    let hour_end_epoch = hour_start_epoch + 3_600;
    sqlx::query("DELETE FROM parallel_work_hourly_rollup WHERE hour_start_epoch = ?1")
        .bind(hour_start_epoch)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM parallel_work_upstream_account_hourly_rollup WHERE hour_start_epoch = ?1",
    )
    .bind(hour_start_epoch)
    .execute(&mut *tx)
    .await?;

    for (source_scope, source_filter) in [
        (PARALLEL_WORK_SCOPE_ALL, None),
        (PARALLEL_WORK_SCOPE_PROXY_ONLY, Some(SOURCE_PROXY)),
    ] {
        let global_sql = if source_filter.is_some() {
            r#"
            INSERT INTO parallel_work_hourly_rollup (
                hour_start_epoch, source_scope, active_minute_count, parallel_count_sum
            )
            SELECT ?1, ?2, COUNT(*), SUM(parallel_count)
            FROM (
                SELECT minute_start_epoch, COUNT(DISTINCT prompt_cache_key) AS parallel_count
                FROM parallel_work_minute_key_rollup
                WHERE minute_start_epoch >= ?3 AND minute_start_epoch < ?4 AND source = ?5
                GROUP BY minute_start_epoch
            )
            HAVING COUNT(*) > 0
            "#
        } else {
            r#"
            INSERT INTO parallel_work_hourly_rollup (
                hour_start_epoch, source_scope, active_minute_count, parallel_count_sum
            )
            SELECT ?1, ?2, COUNT(*), SUM(parallel_count)
            FROM (
                SELECT minute_start_epoch, COUNT(DISTINCT prompt_cache_key) AS parallel_count
                FROM parallel_work_minute_key_rollup
                WHERE minute_start_epoch >= ?3 AND minute_start_epoch < ?4
                GROUP BY minute_start_epoch
            )
            HAVING COUNT(*) > 0
            "#
        };
        let mut global = sqlx::query(global_sql)
            .bind(hour_start_epoch)
            .bind(source_scope)
            .bind(hour_start_epoch)
            .bind(hour_end_epoch);
        if let Some(source) = source_filter {
            global = global.bind(source);
        }
        global.execute(&mut *tx).await?;

        let account_sql = if source_filter.is_some() {
            r#"
            INSERT INTO parallel_work_upstream_account_hourly_rollup (
                hour_start_epoch, source_scope, upstream_account_id,
                active_minute_count, parallel_count_sum
            )
            SELECT ?1, ?2, upstream_account_id, COUNT(*), SUM(parallel_count)
            FROM (
                SELECT minute_start_epoch, upstream_account_id,
                    COUNT(DISTINCT prompt_cache_key) AS parallel_count
                FROM parallel_work_upstream_account_minute_key_rollup
                WHERE minute_start_epoch >= ?3 AND minute_start_epoch < ?4 AND source = ?5
                GROUP BY minute_start_epoch, upstream_account_id
            )
            GROUP BY upstream_account_id
            "#
        } else {
            r#"
            INSERT INTO parallel_work_upstream_account_hourly_rollup (
                hour_start_epoch, source_scope, upstream_account_id,
                active_minute_count, parallel_count_sum
            )
            SELECT ?1, ?2, upstream_account_id, COUNT(*), SUM(parallel_count)
            FROM (
                SELECT minute_start_epoch, upstream_account_id,
                    COUNT(DISTINCT prompt_cache_key) AS parallel_count
                FROM parallel_work_upstream_account_minute_key_rollup
                WHERE minute_start_epoch >= ?3 AND minute_start_epoch < ?4
                GROUP BY minute_start_epoch, upstream_account_id
            )
            GROUP BY upstream_account_id
            "#
        };
        let mut account = sqlx::query(account_sql)
            .bind(hour_start_epoch)
            .bind(source_scope)
            .bind(hour_start_epoch)
            .bind(hour_end_epoch);
        if let Some(source) = source_filter {
            account = account.bind(source);
        }
        account.execute(&mut *tx).await?;

        sqlx::query(
            r#"
            INSERT INTO parallel_work_hourly_coverage (
                hour_start_epoch, source_scope, minute_keys_complete, hourly_scalar_complete
            )
            VALUES (?1, ?2, 1, 1)
            ON CONFLICT(hour_start_epoch, source_scope) DO UPDATE SET
                minute_keys_complete = 1,
                hourly_scalar_complete = 1
            "#,
        )
        .bind(hour_start_epoch)
        .bind(source_scope)
        .execute(&mut *tx)
        .await?;
    }

    for table in [
        "parallel_work_minute_key_rollup",
        "parallel_work_upstream_account_minute_key_rollup",
    ] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2"
        ))
        .bind(hour_start_epoch)
        .bind(hour_end_epoch)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub(crate) async fn rebuild_parallel_work_rollups_for_hours_tx(
    tx: &mut SqliteConnection,
    hour_start_epochs: &[i64],
) -> Result<()> {
    let keep_start_epoch = parallel_work_minute_rollup_keep_start_epoch(Utc::now())?;
    let full_detail_start_epoch = sqlx::query_scalar(
        "SELECT full_detail_start_epoch FROM parallel_work_rollup_coverage_state WHERE id = 1",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let mut hours = hour_start_epochs.to_vec();
    hours.sort_unstable();
    hours.dedup();
    for hour_start_epoch in hours {
        let rebuilt = replace_parallel_work_minute_keys_for_hour_tx(
            tx,
            hour_start_epoch,
            full_detail_start_epoch,
        )
        .await?;
        if rebuilt {
            mark_parallel_work_minute_coverage_tx(tx, hour_start_epoch).await?;
        }
        if rebuilt && hour_start_epoch < keep_start_epoch {
            materialize_parallel_work_hour_tx(tx, hour_start_epoch).await?;
        }
    }
    Ok(())
}

pub(crate) async fn maintain_parallel_work_rollups(
    pool: &Pool<Sqlite>,
    current_full_detail_start_epoch: Option<i64>,
) -> Result<()> {
    let now = Utc::now();
    let keep_start_epoch = parallel_work_minute_rollup_keep_start_epoch(now)?;
    let closed_hour_end = current_hour_start_epoch(now);
    let full_detail_start_epoch =
        advance_parallel_work_full_detail_start_epoch(pool, current_full_detail_start_epoch)
            .await?;
    let initial_hour_epoch = full_detail_start_epoch
        .map(|epoch| epoch.div_euclid(3_600) * 3_600)
        .map(|epoch| epoch.max(keep_start_epoch));

    let expired_hours: Vec<i64> = sqlx::query_scalar(
        r#"
        SELECT hour_start_epoch
        FROM parallel_work_hourly_coverage
        WHERE source_scope = ?1
            AND minute_keys_complete = 1
            AND hourly_scalar_complete = 0
            AND hour_start_epoch < ?2
        ORDER BY hour_start_epoch
        LIMIT ?3
        "#,
    )
    .bind(PARALLEL_WORK_SCOPE_ALL)
    .bind(keep_start_epoch)
    .bind(PARALLEL_WORK_ROLLUP_MAINTENANCE_HOUR_LIMIT)
    .fetch_all(pool)
    .await?;
    for hour_start_epoch in &expired_hours {
        let mut hour_tx = pool.begin().await?;
        materialize_parallel_work_hour_tx(hour_tx.as_mut(), *hour_start_epoch).await?;
        hour_tx.commit().await?;
    }

    // Existing verified minute keys must remain repairable even when the caller
    // has no live-raw coverage window to advance.
    let Some(initial_hour_epoch) = initial_hour_epoch else {
        return Ok(());
    };
    let mut state_tx = pool.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO parallel_work_rollup_maintenance_state (id, next_hour_epoch) VALUES (1, ?1)",
    )
    .bind(initial_hour_epoch)
    .execute(state_tx.as_mut())
    .await?;
    let mut next_hour_epoch: i64 = sqlx::query_scalar(
        "SELECT next_hour_epoch FROM parallel_work_rollup_maintenance_state WHERE id = 1",
    )
    .fetch_one(state_tx.as_mut())
    .await?;
    if next_hour_epoch < initial_hour_epoch {
        sqlx::query(
            "UPDATE parallel_work_rollup_maintenance_state SET next_hour_epoch = ?1 WHERE id = 1",
        )
        .bind(initial_hour_epoch)
        .execute(state_tx.as_mut())
        .await?;
        next_hour_epoch = initial_hour_epoch;
    }
    state_tx.commit().await?;

    let remaining_hours =
        PARALLEL_WORK_ROLLUP_MAINTENANCE_HOUR_LIMIT.saturating_sub(expired_hours.len() as i64);
    for _ in 0..remaining_hours {
        if next_hour_epoch >= closed_hour_end {
            break;
        }
        let mut hour_tx = pool.begin().await?;
        let rebuilt = replace_parallel_work_minute_keys_for_hour_tx(
            hour_tx.as_mut(),
            next_hour_epoch,
            full_detail_start_epoch,
        )
        .await?;
        if rebuilt {
            mark_parallel_work_minute_coverage_tx(hour_tx.as_mut(), next_hour_epoch).await?;
        }
        if rebuilt && next_hour_epoch < keep_start_epoch {
            materialize_parallel_work_hour_tx(hour_tx.as_mut(), next_hour_epoch).await?;
        }
        let following_hour_epoch = next_hour_epoch + 3_600;
        sqlx::query(
            "UPDATE parallel_work_rollup_maintenance_state SET next_hour_epoch = ?1 WHERE id = 1",
        )
        .bind(following_hour_epoch)
        .execute(hour_tx.as_mut())
        .await?;
        hour_tx.commit().await?;
        next_hour_epoch = following_hour_epoch;
    }
    Ok(())
}
