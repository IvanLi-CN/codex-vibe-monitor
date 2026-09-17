use super::*;

pub(crate) async fn legacy_raw_blob_link_seed_completed(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<bool> {
    let migration_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
    )
    .bind("proxy_raw_payload_blob_link_migrations")
    .fetch_one(tx.as_mut())
    .await?
        != 0;
    if !migration_table_exists {
        return Ok(false);
    }

    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM proxy_raw_payload_blob_link_migrations WHERE migration_name = ?1)",
    )
    .bind(LEGACY_RAW_BLOB_LINK_SEED_MIGRATION_NAME)
    .fetch_one(tx.as_mut())
    .await?
        != 0)
}

pub(crate) fn legacy_archive_segment_id_range(part_key: &str) -> Option<(i64, i64)> {
    let encoded = part_key.strip_prefix("part-")?;
    let (lower, remainder) = encoded.split_once('-')?;
    let (upper, _) = remainder.split_once('-')?;
    let lower = i64::from_str_radix(lower, 16).ok()?;
    let upper = i64::from_str_radix(upper, 16).ok()?;
    (lower > 0 && upper >= lower).then_some((lower, upper))
}

pub(crate) async fn classify_legacy_invocation_detail_archive_mirrors(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    const CLASSIFICATION_CHUNK_SIZE: i64 = 512;
    let mut after_id = 0_i64;

    loop {
        let candidates = sqlx::query_as::<_, (i64, String)>(
            r#"
            SELECT id, part_key
            FROM archive_batches
            WHERE dataset = 'codex_invocations'
              AND status = 'completed'
              AND summary_source_kind = 'unknown'
              AND layout = 'segment_v1'
              AND part_key IS NOT NULL
              AND id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(after_id)
        .bind(CLASSIFICATION_CHUNK_SIZE)
        .fetch_all(pool)
        .await?;
        let Some(last_id) = candidates.last().map(|(id, _)| *id) else {
            break;
        };
        after_id = last_id;

        let ranges = candidates
            .into_iter()
            .filter_map(|(id, part_key)| {
                legacy_archive_segment_id_range(&part_key)
                    .map(|(lower_id, upper_id)| (id, lower_id, upper_id))
            })
            .collect::<Vec<_>>();
        if ranges.is_empty() {
            continue;
        }

        let mut query = sqlx::QueryBuilder::<Sqlite>::new(
            "WITH candidates(id, lower_id, upper_id) AS (VALUES ",
        );
        for (index, (id, lower_id, upper_id)) in ranges.into_iter().enumerate() {
            if index > 0 {
                query.push(", ");
            }
            query
                .push("(")
                .push_bind(id)
                .push(", ")
                .push_bind(lower_id)
                .push(", ")
                .push_bind(upper_id)
                .push(")");
        }
        query.push(
            ") \
             UPDATE archive_batches \
             SET summary_source_kind = 'live_mirror' \
             WHERE summary_source_kind = 'unknown' \
               AND id IN ( \
                    SELECT id FROM candidates \
                    WHERE ( \
                        SELECT COUNT(*) FROM codex_invocations AS live \
                        WHERE live.id BETWEEN lower_id AND upper_id \
                    ) = upper_id - lower_id + 1 \
               )",
        );
        query.build().execute(pool).await?;
    }

    Ok(())
}
