use super::*;

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
