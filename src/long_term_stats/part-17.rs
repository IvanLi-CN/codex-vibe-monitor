fn long_term_projection_hourly_retention_start_date(retention_days: u64) -> NaiveDate {
    Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(retention_days.max(366) as i64 - 1)
}

async fn prune_long_term_projection_hourly_retention(
    pool: &Pool<Sqlite>,
    retention_days: u64,
) -> Result<(u64, u64)> {
    let control = LongTermProjectionWriteControl::unrestricted();
    prune_long_term_projection_hourly_retention_with_control(pool, retention_days, &control).await
}

#[derive(Debug, Clone, Copy)]
enum LongTermProjectionRetentionTarget {
    HourlyRollup,
    LegacyInterval,
    CanonicalInterval,
    Suppression,
    RebuildMember,
}

async fn long_term_projection_hourly_retention_target(
    pool: &Pool<Sqlite>,
    retention_start_date: NaiveDate,
    retention_start_epoch: i64,
) -> Result<Option<LongTermProjectionRetentionTarget>> {
    let retention_start_ms = retention_start_epoch * 1_000;
    let candidates = [
        (
            LongTermProjectionRetentionTarget::HourlyRollup,
            "SELECT EXISTS(SELECT 1 FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT 1)",
            retention_start_epoch,
        ),
        (
            LongTermProjectionRetentionTarget::LegacyInterval,
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_intervals WHERE bucket_kind = 'hourly' AND bucket_date < ?1 LIMIT 1)",
            0,
        ),
        (
            LongTermProjectionRetentionTarget::CanonicalInterval,
            "SELECT EXISTS(SELECT 1 FROM long_term_projection_interval_state WHERE interval_end_ms < ?1 LIMIT 1)",
            retention_start_ms,
        ),
    ];
    for (target, statement, value) in candidates {
        let mut query = sqlx::query_scalar::<_, i64>(statement);
        if matches!(target, LongTermProjectionRetentionTarget::LegacyInterval) {
            query = query.bind(retention_start_date.to_string());
        } else {
            query = query.bind(value);
        }
        if query.fetch_one(pool).await? != 0 {
            return Ok(Some(target));
        }
    }
    for (target, table) in [
        (
            LongTermProjectionRetentionTarget::Suppression,
            "long_term_projection_interval_suppressions",
        ),
        (
            LongTermProjectionRetentionTarget::RebuildMember,
            "long_term_projection_rebuild_members",
        ),
    ] {
        let statement = format!(
            "SELECT EXISTS(SELECT 1 FROM {table} metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT 1)"
        );
        if sqlx::query_scalar::<_, i64>(&statement)
            .fetch_one(pool)
            .await?
            != 0
        {
            return Ok(Some(target));
        }
    }
    Ok(None)
}

async fn prune_long_term_projection_hourly_retention_with_control(
    pool: &Pool<Sqlite>,
    retention_days: u64,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<(u64, u64)> {
    let retention_start_date = long_term_projection_hourly_retention_start_date(retention_days);
    let retention_start_epoch = retention_start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .map(|value| value.timestamp())
        .context("invalid long-term projection hourly retention start")?;
    let Some(target) = long_term_projection_hourly_retention_target(
        pool,
        retention_start_date,
        retention_start_epoch,
    )
    .await?
    else {
        return Ok((0, 0));
    };

    let (mut tx, permit) = control.begin(pool).await?;
    let deleted = match target {
        LongTermProjectionRetentionTarget::HourlyRollup => {
            sqlx::query(
                "DELETE FROM long_term_usage_hourly WHERE rowid IN (SELECT rowid FROM long_term_usage_hourly WHERE bucket_start_epoch < ?1 LIMIT ?2)",
            )
            .bind(retention_start_epoch)
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::LegacyInterval => {
            sqlx::query(
                "DELETE FROM long_term_projection_intervals WHERE rowid IN (SELECT rowid FROM long_term_projection_intervals WHERE bucket_kind = 'hourly' AND bucket_date < ?1 LIMIT ?2)",
            )
            .bind(retention_start_date.to_string())
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::CanonicalInterval => {
            sqlx::query(
                "DELETE FROM long_term_projection_interval_state WHERE rowid IN (SELECT rowid FROM long_term_projection_interval_state WHERE interval_end_ms < ?1 LIMIT ?2)",
            )
            .bind(retention_start_epoch * 1_000)
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::Suppression => {
            sqlx::query(
                "DELETE FROM long_term_projection_interval_suppressions WHERE rowid IN (SELECT metadata.rowid FROM long_term_projection_interval_suppressions metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT ?1)",
            )
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
        LongTermProjectionRetentionTarget::RebuildMember => {
            sqlx::query(
                "DELETE FROM long_term_projection_rebuild_members WHERE rowid IN (SELECT metadata.rowid FROM long_term_projection_rebuild_members metadata WHERE NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = metadata.invocation_row_id) LIMIT ?1)",
            )
            .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        }
    };
    control.commit(tx, permit).await?;
    Ok(match target {
        LongTermProjectionRetentionTarget::HourlyRollup => (deleted, 0),
        LongTermProjectionRetentionTarget::LegacyInterval
        | LongTermProjectionRetentionTarget::CanonicalInterval
        | LongTermProjectionRetentionTarget::Suppression
        | LongTermProjectionRetentionTarget::RebuildMember => (0, deleted),
    })
}

async fn load_long_term_terminal_watermark(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(id), 0) FROM codex_invocations WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
    )
    .fetch_one(pool)
    .await?)
}

async fn load_long_term_projection_cursor(pool: &Pool<Sqlite>) -> Result<i64> {
    let control = LongTermProjectionWriteControl::unrestricted();
    load_long_term_projection_cursor_with_control(pool, &control).await
}

async fn load_long_term_projection_cursor_with_control(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<i64> {
    if let Some(cursor) = control
        .await_sqlite(
            sqlx::query_as::<_, LongTermProjectionCursorRow>(
                "SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_optional(pool),
            None,
        )
        .await?
    {
        return Ok(cursor.cursor_row_id);
    }

    let pressure_permit = control.try_begin_background()?;
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let write_permit = coordinator
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        .ok_or_else(|| {
            anyhow!(
                "long-term projection write deferred by database pressure: P2 cursor initialization is not admitted"
            )
        })?;

    // Another process can initialize the durable cursor between the initial read and this
    // short write. Keep that successful race on the read-only path.
    if let Some(cursor) = control
        .await_sqlite(
            sqlx::query_as::<_, LongTermProjectionCursorRow>(
                "SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_optional(pool),
            Some(&coordinator),
        )
        .await?
    {
        return Ok(cursor.cursor_row_id);
    }

    let mut tx = control
        .begin_cursor_initialization(pool, &coordinator)
        .await?;
    if coordinator.p2_should_yield() {
        drop(tx);
        bail!(
            "long-term projection write deferred by database pressure: P2 cursor initialization yielded to higher-priority work"
        );
    }
    sqlx::query(
        "INSERT OR IGNORE INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .execute(&mut *tx)
    .await?;
    control
        .commit(
            tx,
            Some(LongTermProjectionWritePermit {
                _pressure: pressure_permit,
                _write: Some(write_permit),
            }),
        )
        .await?;
    Ok(control
        .await_sqlite(
            sqlx::query_as::<_, LongTermProjectionCursorRow>(
                "SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_one(pool),
            None,
        )
        .await?
        .cursor_row_id)
}

#[derive(Debug)]
struct LongTermProjectionDateRebuild {
    bucket_date: String,
    start_epoch: i64,
    end_epoch: i64,
    hourly: HashMap<(i64, String, String), LongTermBucket>,
    daily: HashMap<(String, String, String), LongTermBucket>,
    interval_segments: Vec<LongTermProjectionIntervalSegment>,
    source_row_count: u64,
}

#[derive(Debug)]
struct LongTermProjectionRebuildPublication<'a> {
    next_cursor: Option<i64>,
    clear_dirty_buckets: &'a [LongTermProjectionDirtyBucket],
    mark_ready: bool,
    publish_state: bool,
    publication_token: Option<&'a str>,
    repaired_start_date: Option<&'a str>,
}

#[derive(Debug, Clone, FromRow)]
struct LongTermProjectionDirtyBucket {
    bucket_date: String,
    generation: i64,
}

#[derive(Debug, FromRow)]
struct LongTermProjectionPublicationMember {
    bucket_date: String,
    rebuild_token: String,
    publication_token: String,
    publication_generation: Option<i64>,
}

fn next_long_term_projection_publication_token() -> String {
    let sequence =
        LONG_TERM_PROJECTION_PUBLICATION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    format!(
        "long-term-publication:{}:{sequence}",
        Utc::now().timestamp_micros()
    )
}

fn long_term_projection_row_affects_date(row: &LongTermInvocationRow, bucket_date: &str) -> bool {
    let mut hourly = HashMap::new();
    let mut daily = HashMap::new();
    let mut statistics_start = None;
    accumulate_long_term_invocation(row, &mut hourly, &mut daily, &mut statistics_start);
    daily.keys().any(|(date, _, _)| date == bucket_date)
}
