use super::*;

pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_LENGTH: usize = 8;
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_RETRY_LIMIT: usize = 16;
const POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_ALPHABET: [char; 58] = [
    '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K',
    'L', 'M', 'N', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd', 'e',
    'f', 'g', 'h', 'i', 'j', 'k', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y',
    'z',
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PoolAttemptPublicIdBackfillSummary {
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
}

pub(crate) fn pool_upstream_request_attempt_public_id_has_alpha(value: &str) -> bool {
    value.chars().any(|char| char.is_ascii_alphabetic())
}

pub(crate) fn generate_pool_upstream_request_attempt_public_id() -> String {
    loop {
        let candidate = nanoid::nanoid!(
            POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_LENGTH,
            &POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_ALPHABET
        );
        if pool_upstream_request_attempt_public_id_has_alpha(&candidate) {
            return candidate;
        }
    }
}

pub(crate) fn is_pool_upstream_request_attempt_public_id_collision(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(database_error) = error else {
        return false;
    };
    let code_matches = database_error
        .code()
        .as_deref()
        .is_some_and(|code| code == "1555" || code == "2067");
    let message = database_error.message().to_ascii_lowercase();
    code_matches
        && (message.contains("attempt_public_id")
            || message.contains("idx_pool_upstream_request_attempts_public_id"))
}

pub(crate) async fn assign_pool_upstream_request_attempt_public_id_if_missing(
    conn: &mut SqliteConnection,
    attempt_row_id: i64,
) -> Result<bool> {
    for _ in 0..POOL_UPSTREAM_REQUEST_ATTEMPT_PUBLIC_ID_RETRY_LIMIT {
        let attempt_public_id = generate_pool_upstream_request_attempt_public_id();
        match sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET attempt_public_id = ?1
            WHERE id = ?2
              AND TRIM(COALESCE(attempt_public_id, '')) = ''
            "#,
        )
        .bind(&attempt_public_id)
        .bind(attempt_row_id)
        .execute(&mut *conn)
        .await
        {
            Ok(result) => return Ok(result.rows_affected() > 0),
            Err(error) if is_pool_upstream_request_attempt_public_id_collision(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }

    bail!(
        "failed to allocate unique attempt_public_id for pool_upstream_request_attempts row {}",
        attempt_row_id
    );
}

pub(crate) async fn backfill_pool_upstream_request_attempt_public_ids_on_connection(
    conn: &mut SqliteConnection,
    start_after_id: i64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
    publish_analytics_changes: bool,
) -> Result<BackfillBatchOutcome<PoolAttemptPublicIdBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = PoolAttemptPublicIdBackfillSummary::default();
    let mut examined = 0_u64;
    let mut last_seen_id = start_after_id;
    let mut committed_cursor_id = start_after_id;
    let mut committed_scanned = 0_u64;
    let mut committed_updated = 0_u64;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, examined, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let window_start_id = last_seen_id;
        let window_ids = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM pool_upstream_request_attempts WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
        )
        .bind(window_start_id)
        .bind(startup_backfill_query_limit(examined, scan_limit))
        .fetch_all(&mut *conn)
        .await
        .map_err(|error| {
            anyhow::Error::new(crate::BackfillPartialFailure {
                source: error.into(),
                next_cursor_id: committed_cursor_id,
                scanned: committed_scanned,
                updated: committed_updated,
            })
        })?;
        let Some(window_end_id) = window_ids.last().copied() else {
            break;
        };
        examined = examined.saturating_add(window_ids.len() as u64);
        last_seen_id = window_end_id;

        let rows = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM pool_upstream_request_attempts
            WHERE id > ?1
              AND id <= ?2
              AND TRIM(COALESCE(attempt_public_id, '')) = ''
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(window_start_id)
        .bind(window_end_id)
        .bind(window_ids.len() as i64)
        .fetch_all(&mut *conn)
        .await
        .map_err(|error| {
            anyhow::Error::new(crate::BackfillPartialFailure {
                source: error.into(),
                next_cursor_id: committed_cursor_id,
                scanned: committed_scanned,
                updated: committed_updated,
            })
        })?;

        if rows.is_empty() {
            committed_cursor_id = window_end_id;
            committed_scanned = summary.scanned;
            committed_updated = summary.updated;
            continue;
        }

        let mut transaction = conn.begin().await.map_err(|error| {
            anyhow::Error::new(crate::BackfillPartialFailure {
                source: error.into(),
                next_cursor_id: committed_cursor_id,
                scanned: committed_scanned,
                updated: committed_updated,
            })
        })?;
        for row_id in rows {
            summary.scanned += 1;
            let updated =
                assign_pool_upstream_request_attempt_public_id_if_missing(&mut transaction, row_id)
                    .await
                    .map_err(|error| {
                        anyhow::Error::new(crate::BackfillPartialFailure {
                            source: error,
                            next_cursor_id: committed_cursor_id,
                            scanned: committed_scanned,
                            updated: committed_updated,
                        })
                    })?;
            if updated {
                summary.updated += 1;
                push_backfill_sample(&mut samples, format!("id={row_id}"));
                if publish_analytics_changes {
                    crate::schema::analytics_changes::append_attempt_upsert_for_row_tx(
                        &mut transaction,
                        row_id,
                    )
                    .await
                    .map_err(|error| {
                        anyhow::Error::new(crate::BackfillPartialFailure {
                            source: error,
                            next_cursor_id: committed_cursor_id,
                            scanned: committed_scanned,
                            updated: committed_updated,
                        })
                    })?;
                }
            }
        }
        transaction.commit().await.map_err(|error| {
            anyhow::Error::new(crate::BackfillPartialFailure {
                source: error.into(),
                next_cursor_id: committed_cursor_id,
                scanned: committed_scanned,
                updated: committed_updated,
            })
        })?;
        committed_cursor_id = window_end_id;
        committed_scanned = summary.scanned;
        committed_updated = summary.updated;
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: committed_cursor_id,
        hit_budget,
        samples,
    })
}

pub(crate) async fn backfill_pool_upstream_request_attempt_public_ids_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<PoolAttemptPublicIdBackfillSummary>> {
    let mut conn = crate::observability::diagnostics::wait(
        crate::observability::diagnostics::Resource::DbPool,
        pool.acquire(),
    )
    .await?;
    backfill_pool_upstream_request_attempt_public_ids_on_connection(
        &mut conn,
        start_after_id,
        scan_limit,
        max_elapsed,
        true,
    )
    .await
}
