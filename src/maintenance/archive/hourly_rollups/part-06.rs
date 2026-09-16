pub(crate) async fn rebuild_upstream_account_stats_rollups_from_sources(
    pool: &Pool<Sqlite>,
) -> Result<(usize, usize)> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut seen_ids = HashSet::new();
    let mut source_rows = Vec::<InvocationHourlySourceRecord>::new();
    let mut source_incomplete = false;

    for archive_file in archive_files {
        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during upstream account stats rollup rebuild"
            );
            source_incomplete = true;
            continue;
        }

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        let archive_query_sql = build_legacy_compatible_invocation_archive_query(&archive_columns);
        let mut archive_cursor_id = 0_i64;
        loop {
            let mut archive_rows =
                sqlx::query_as::<_, InvocationHourlySourceRecord>(&archive_query_sql)
                    .bind(archive_cursor_id)
                    .bind(BACKFILL_BATCH_SIZE)
                    .fetch_all(&archive_pool)
                    .await?;
            if archive_rows.is_empty() {
                break;
            }
            archive_cursor_id = archive_rows
                .last()
                .map(|row| row.id)
                .unwrap_or(archive_cursor_id);
            archive_rows.retain(|row| seen_ids.insert(row.id));
            if archive_rows
                .iter()
                .any(|row| !row.has_complete_token_components())
            {
                source_incomplete = true;
                warn!(
                    dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    file_path = %archive_path.display(),
                    "skipping upstream account stats rollup rebuild because archive token components are incomplete"
                );
                break;
            }
            source_rows.extend(archive_rows);
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    let mut cursor_id = 0_i64;
    let mut live_conn = pool.acquire().await?;
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(&mut live_conn).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(&mut live_conn).await?;
    loop {
        let mut live_rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                {} AS upstream_account_id,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                {} AS first_token_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms
            FROM codex_invocations
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
            upstream_account_id_sql, first_token_ms_sql,
        ))
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(&mut *live_conn)
        .await?;
        if live_rows.is_empty() {
            break;
        }
        cursor_id = live_rows.last().map(|row| row.id).unwrap_or(cursor_id);
        live_rows.retain(|row| seen_ids.insert(row.id));
        if live_rows
            .iter()
            .any(|row| !row.has_complete_token_components())
        {
            source_incomplete = true;
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                "skipping upstream account stats rollup rebuild because live token components are incomplete"
            );
            break;
        }
        source_rows.extend(live_rows);
    }
    drop(live_conn);

    if source_incomplete {
        let hourly_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
                .fetch_one(pool)
                .await?;
        let minute_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
                .fetch_one(pool)
                .await?;
        return Ok((hourly_count.max(0) as usize, minute_count.max(0) as usize));
    }

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM upstream_account_stats_hourly")
        .execute(tx.as_mut())
        .await?;
    sqlx::query("DELETE FROM upstream_account_stats_minute")
        .execute(tx.as_mut())
        .await?;
    if !source_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &source_rows,
            &[
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
            ],
        )
        .await?;
    }
    if cursor_id > 0 {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            cursor_id,
        )
        .await?;
    }
    tx.commit().await?;

    let hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await?;
    let minute_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await?;
    Ok((hourly_count.max(0) as usize, minute_count.max(0) as usize))
}
