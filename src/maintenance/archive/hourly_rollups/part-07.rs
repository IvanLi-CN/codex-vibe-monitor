pub(crate) async fn replay_live_forward_proxy_attempt_hourly_rollups(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_forward_proxy_attempt_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS, last_id)
        .await?;
    Ok(rows.len() as u64)
}

#[derive(Debug, Clone, FromRow)]
struct UpstreamHostNetworkMinuteSourceRow {
    id: i64,
    bucket_start_epoch: i64,
    source: String,
    upstream_base_url_host: String,
    upload_bytes: i64,
    download_bytes: i64,
}

fn upstream_host_network_direct_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) \
         END"
    )
}

fn upstream_host_network_direct_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             CASE \
               WHEN COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) < 0 THEN 0 \
               ELSE COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) \
             END, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             CASE \
               WHEN COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) < 0 THEN 0 \
               ELSE COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) \
             END, \
             0 \
           ) \
         END"
    )
}

fn upstream_host_network_pool_attempt_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) \
         END"
    )
}

fn upstream_host_network_pool_attempt_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) \
         END"
    )
}

fn upstream_host_network_payload_host_sql(alias: &str) -> String {
    format!(
        "COALESCE( \
            NULLIF(LOWER(TRIM(CAST(CASE \
                WHEN json_valid({alias}.payload) AND json_type({alias}.payload, '$.upstreamBaseUrlHost') = 'text' \
                    THEN json_extract({alias}.payload, '$.upstreamBaseUrlHost') \
                WHEN json_valid({alias}.payload) AND json_type({alias}.payload, '$.upstream_base_url_host') = 'text' \
                    THEN json_extract({alias}.payload, '$.upstream_base_url_host') \
            END AS TEXT))), ''), \
            {unknown_host} \
        )",
        unknown_host =
            sql_quote(crate::dashboard_network_speed::DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST),
    )
}

fn upstream_host_network_attempt_host_sql(alias: &str) -> String {
    format!(
        "COALESCE(NULLIF(LOWER(TRIM({alias}.upstream_base_url_host)), ''), {unknown_host})",
        unknown_host =
            sql_quote(crate::dashboard_network_speed::DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST),
    )
}

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

async fn load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    max_id_sql: &str,
) -> Result<(i64, bool)> {
    if crate::stats::hourly_rollup_progress_exists(&mut *tx, dataset).await? {
        return Ok((
            load_hourly_rollup_live_progress_tx(tx, dataset).await?,
            false,
        ));
    }
    let cursor_id = sqlx::query_scalar::<_, Option<i64>>(max_id_sql)
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or(0)
        .max(0);
    save_hourly_rollup_live_progress_tx(tx, dataset, cursor_id).await?;
    Ok((cursor_id, true))
}

async fn upsert_upstream_host_network_minute_rows_tx(
    tx: &mut SqliteConnection,
    rows: &[UpstreamHostNetworkMinuteSourceRow],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut aggregates = BTreeMap::<
        (i64, String, String),
        crate::dashboard_network_speed::DashboardNetworkByteTotals,
    >::new();
    for row in rows {
        let entry = aggregates
            .entry((
                row.bucket_start_epoch,
                row.source.clone(),
                row.upstream_base_url_host.clone(),
            ))
            .or_default();
        entry.upload_bytes = entry.upload_bytes.saturating_add(row.upload_bytes.max(0));
        entry.download_bytes = entry
            .download_bytes
            .saturating_add(row.download_bytes.max(0));
    }

    for ((bucket_start_epoch, source, upstream_base_url_host), totals) in aggregates {
        sqlx::query(
            r#"
            INSERT INTO upstream_host_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upload_bytes,
                download_bytes,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
            ON CONFLICT(bucket_start_epoch, source, upstream_base_url_host) DO UPDATE SET
                upload_bytes = upstream_host_network_minute.upload_bytes + excluded.upload_bytes,
                download_bytes = upstream_host_network_minute.download_bytes + excluded.download_bytes,
                updated_at = datetime('now')
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .bind(upstream_base_url_host)
        .bind(totals.upload_bytes.max(0))
        .bind(totals.download_bytes.max(0))
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn replay_live_upstream_host_network_minute_rollups_from_invocations(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let (cursor_id, seeded) = load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT,
        "SELECT MAX(id) FROM codex_invocations",
    )
    .await?;
    if seeded {
        tx.commit().await?;
        return Ok(0);
    }

    let upload_bytes_sql = upstream_host_network_direct_upload_bytes_sql("inv");
    let download_bytes_sql = upstream_host_network_direct_download_bytes_sql("inv");
    let host_sql = upstream_host_network_payload_host_sql("inv");
    let rows = sqlx::query_as::<_, UpstreamHostNetworkMinuteSourceRow>(
        format!(
            r#"
            SELECT
                inv.id,
                ((unixepoch(inv.occurred_at || '+08:00') / 60) * 60) AS bucket_start_epoch,
                inv.source,
                {host_sql} AS upstream_base_url_host,
                {upload_bytes_sql} AS upload_bytes,
                {download_bytes_sql} AS download_bytes
            FROM codex_invocations AS inv
            WHERE inv.id > ?1
              AND (
                    COALESCE(
                        CASE
                            WHEN json_valid(inv.payload)
                                THEN TRIM(CAST(json_extract(inv.payload, '$.routeMode') AS TEXT))
                        END,
                        ''
                    ) <> ?2
                    OR NOT EXISTS (
                        SELECT 1
                        FROM pool_upstream_request_attempts AS attempts
                        WHERE attempts.invoke_id = inv.invoke_id
                          AND attempts.occurred_at = inv.occurred_at
                    )
              )
            ORDER BY inv.id ASC
            LIMIT ?3
            "#,
        )
        .as_str(),
    )
    .bind(cursor_id)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_upstream_host_network_minute_rows_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_upstream_host_network_minute_rollups_from_pool_attempts(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let (cursor_id, seeded) = load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS,
        "SELECT MAX(id) FROM pool_upstream_request_attempts",
    )
    .await?;
    if seeded {
        tx.commit().await?;
        return Ok(0);
    }

    let upload_bytes_sql = upstream_host_network_pool_attempt_upload_bytes_sql("attempts");
    let download_bytes_sql = upstream_host_network_pool_attempt_download_bytes_sql("attempts");
    let host_sql = upstream_host_network_attempt_host_sql("attempts");
    let rows = sqlx::query_as::<_, UpstreamHostNetworkMinuteSourceRow>(
        format!(
            r#"
            SELECT
                attempts.id,
                ((unixepoch(attempts.occurred_at || '+08:00') / 60) * 60) AS bucket_start_epoch,
                inv.source,
                {host_sql} AS upstream_base_url_host,
                {upload_bytes_sql} AS upload_bytes,
                {download_bytes_sql} AS download_bytes
            FROM pool_upstream_request_attempts AS attempts
            INNER JOIN codex_invocations AS inv
                ON inv.invoke_id = attempts.invoke_id
               AND inv.occurred_at = attempts.occurred_at
            WHERE attempts.id > ?1
            ORDER BY attempts.id ASC
            LIMIT ?2
            "#,
        )
        .as_str(),
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_upstream_host_network_minute_rows_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvocationHourlyRollupReconciliation {
    pub(crate) applied_rollups: usize,
    /// A complete source scan omitted a canonical bucket inside the reconstructable source
    /// window. The stale bucket is removed and callers use these dates to repair dependent
    /// long-term materializations, including an empty replacement when appropriate.
    pub(crate) invalidated_bucket_start_epochs: Vec<i64>,
    /// Completed invocation archives that could not participate in the source scan. Consumers
    /// must invalidate their own replay markers so a restored file is read again.
    pub(crate) unavailable_archive_file_paths: Vec<String>,
    pub(crate) source_complete: bool,
}

#[derive(Debug, Clone, FromRow)]
struct InvocationArchiveIntegrityFileRow {
    file_path: String,
    sha256: String,
}

async fn load_long_term_integrity_source_start_date(
    connection: &mut SqliteConnection,
) -> Result<Option<NaiveDate>> {
    let source_start = match sqlx::query_scalar::<_, Option<String>>(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_optional(connection)
    .await
    {
        Ok(Some(Some(value))) => value,
        Ok(Some(None) | None) => return Ok(None),
        // Invocation hourly rollups can be reconciled during schema recovery before the
        // long-term state table exists. Without a durable boundary, preserve the existing
        // conservative behavior and require every trusted bucket to be present in sources.
        Err(error)
            if error.to_string().contains("no such table")
                || error.to_string().contains("no such column") =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    NaiveDate::parse_from_str(&source_start, "%Y-%m-%d")
        .map(Some)
        .map_err(|error| {
            anyhow!("long-term integrity source boundary is invalid ({source_start}): {error}")
        })
}

fn long_term_integrity_source_boundary_excludes_bucket(
    bucket_start_epoch: i64,
    source_start: Option<&NaiveDate>,
) -> bool {
    let Some(source_start) = source_start else {
        return false;
    };
    Shanghai
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .is_some_and(|bucket_start| bucket_start.date_naive() < *source_start)
}

fn long_term_integrity_source_boundary_start_epoch(source_start: &NaiveDate) -> Result<i64> {
    let midnight = source_start
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("long-term integrity source boundary has no local midnight"))?;
    Shanghai
        .from_local_datetime(&midnight)
        .single()
        .map(|value| value.timestamp())
        .ok_or_else(|| anyhow!("long-term integrity source boundary midnight is ambiguous"))
}
