/// Reads every input needed by the hot path while running off-request. The resulting projection
/// is swapped atomically in the subscription hub only after the complete baseline is available.
async fn build_summary_projection(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
    previous_all_time: Option<PreviousSummaryProjectionAllTime>,
    durable_terminal_sequence_watermark: u64,
) -> Result<SummaryProjection> {
    #[cfg(test)]
    if state.config.database_path == std::path::Path::new(":memory:") {
        // Stateful unit fixtures use a private shared-memory URI while their AppConfig retains
        // `:memory:`. A second URI would point at a blank database, so exercise the regular
        // bounded builder directly in those fixtures.
        return build_summary_projection_once(
            state,
            &state.pool,
            mode,
            previous_all_time,
            durable_terminal_sequence_watermark,
        )
        .await;
    }

    // Hydration issues hundreds of bounded queries. Keep them on one read transaction so a
    // retention commit cannot mix an old rollup with a replacement manifest. WAL readers do not
    // block terminal writers, unlike a global `data_version` retry which treats any new request
    // as a source invalidation and can leave the first snapshot unavailable under normal load.
    let snapshot_options = SqliteConnectOptions::from_str(&state.config.database_url())
        .context("summary projection snapshot connection options failed")?
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS));
    let snapshot_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(snapshot_options)
        .await
        .context("summary projection snapshot connection failed")?;
    let mut connection = snapshot_pool
        .acquire()
        .await
        .context("summary projection snapshot acquisition failed")?;
    sqlx::query("BEGIN")
        .execute(&mut *connection)
        .await
        .context("summary projection snapshot begin failed")?;
    drop(connection);

    let build = build_summary_projection_once(
        state,
        &snapshot_pool,
        mode,
        previous_all_time,
        durable_terminal_sequence_watermark,
    )
    .await;
    let rollback = sqlx::query("ROLLBACK")
        .execute(&snapshot_pool)
        .await
        .context("summary projection snapshot rollback failed");
    snapshot_pool.close().await;

    match (build, rollback) {
        (Ok(projection), Ok(_)) => Ok(projection),
        (Ok(_), Err(error)) | (Err(error), Ok(_)) | (Err(error), Err(_)) => Err(error),
    }
}
