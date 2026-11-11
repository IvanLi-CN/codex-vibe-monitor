use super::*;

const PROJECTION_CONSUMER: &str = "long_term_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LongTermProjectionMaterializationReport {
    pub(crate) row_count: i64,
    pub(crate) source_cursor_row_id: i64,
    pub(crate) digest: [u8; 32],
}

#[derive(Debug, FromRow)]
struct DailyRow {
    stats_date: String,
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

#[derive(Debug, FromRow)]
struct HourlyRow {
    bucket_start_epoch: i64,
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

#[derive(Debug, FromRow)]
struct IntervalStateRow {
    invocation_row_id: i64,
    model_series_key: String,
    upstream_series_key: String,
    interval_start_ms: i64,
    interval_end_ms: i64,
}

#[derive(Debug)]
struct SourceSnapshot {
    daily: Vec<DailyRow>,
    hourly: Vec<HourlyRow>,
    interval_state: Vec<IntervalStateRow>,
    cursor_row_id: i64,
    digest: [u8; 32],
    row_count: i64,
    owner_kind: String,
    owner_epoch: i64,
    serving_fence: i64,
    generation: i64,
    serving_ack_change_id: i64,
}

#[derive(Debug)]
struct DigestBuilder(Sha256);

impl DigestBuilder {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn text(&mut self, value: &str) {
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value.as_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.0.update(value.to_be_bytes());
    }

    fn f64(&mut self, value: f64) {
        self.0.update(value.to_bits().to_be_bytes());
    }

    fn daily(&mut self, row: &DailyRow) {
        self.text("daily");
        self.text(&row.stats_date);
        self.text(&row.dimension);
        self.text(&row.series_key);
        self.text(&row.display_name);
        self.text(&row.reasoning_effort);
        self.i64(row.calls);
        self.i64(row.token_total);
        self.i64(row.token_samples);
        self.f64((row.cost_total * 100_000_000.0).round() / 100_000_000.0);
        self.i64(row.cost_samples);
        self.f64(row.usage_time_ms);
        self.i64(row.usage_time_samples);
        self.f64(row.wall_time_ms);
        self.i64(row.wall_time_samples);
        self.i64(row.output_tokens_total);
        self.f64(row.stream_duration_ms);
        self.i64(row.output_speed_samples);
        self.f64(row.first_byte_sum_ms);
        self.i64(row.first_byte_samples);
        self.f64(row.response_sum_ms);
        self.i64(row.response_samples);
    }

    fn hourly(&mut self, row: &HourlyRow) {
        self.text("hourly");
        self.i64(row.bucket_start_epoch);
        self.text(&row.dimension);
        self.text(&row.series_key);
        self.text(&row.display_name);
        self.text(&row.reasoning_effort);
        self.i64(row.calls);
        self.i64(row.token_total);
        self.i64(row.token_samples);
        self.f64((row.cost_total * 100_000_000.0).round() / 100_000_000.0);
        self.i64(row.cost_samples);
        self.f64(row.usage_time_ms);
        self.i64(row.usage_time_samples);
        self.f64(row.wall_time_ms);
        self.i64(row.wall_time_samples);
        self.i64(row.output_tokens_total);
        self.f64(row.stream_duration_ms);
        self.i64(row.output_speed_samples);
        self.f64(row.first_byte_sum_ms);
        self.i64(row.first_byte_samples);
        self.f64(row.response_sum_ms);
        self.i64(row.response_samples);
    }

    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

impl PgAnalyticsRuntime {
    pub(crate) async fn invalidate_long_term_projection_for_refresh(&self) -> Result<()> {
        let target = self
            .maintenance_pool
            .as_ref()
            .context("PostgreSQL analytics maintenance pool is unavailable")?;
        let mut tx = target.inner().begin().await?;
        sqlx::query(
            "UPDATE cvm_analytics.analytics_projection_state
             SET status = 'preparing', last_error_code = 'source_refresh_in_progress', updated_at = NOW()
             WHERE id = 1",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE cvm_analytics.analytics_projection_target_status
             SET ready = FALSE, last_error_code = 'source_refresh_in_progress', updated_at = NOW()",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Copy the complete SQLite long-term projection into PostgreSQL atomically.
    /// The target remains fail-closed until row count and digest verification pass.
    pub(crate) async fn materialize_long_term_projection(
        &self,
        source_pool: &Pool<Sqlite>,
    ) -> Result<LongTermProjectionMaterializationReport> {
        let target = self
            .maintenance_pool
            .as_ref()
            .context("PostgreSQL analytics maintenance pool is unavailable")?;
        let snapshot = load_source_snapshot(source_pool).await?;
        let source_fence = (
            snapshot.owner_kind.clone(),
            snapshot.owner_epoch,
            snapshot.serving_fence,
            snapshot.generation,
        );
        if snapshot.owner_kind == "postgres" {
            anyhow::ensure!(
                snapshot.cursor_row_id >= snapshot.serving_ack_change_id,
                "SQLite rollback projection cursor does not cover the PostgreSQL serving acknowledgement"
            );
        }
        sqlx::query(
            "UPDATE cvm_analytics.analytics_projection_state
             SET status = 'preparing', last_error_code = 'snapshot_copy_in_progress', updated_at = NOW()
             WHERE id = 1",
        )
        .execute(target.inner())
        .await?;
        sqlx::query(
            "UPDATE cvm_analytics.analytics_projection_target_status
             SET ready = FALSE, last_error_code = 'snapshot_copy_in_progress', updated_at = NOW()",
        )
        .execute(target.inner())
        .await?;

        let result = copy_snapshot(target.inner(), &snapshot).await;
        if let Err(error) = result {
            let _ = sqlx::query(
                "UPDATE cvm_analytics.analytics_projection_state
                 SET status = 'error', last_error_code = 'snapshot_copy_failed', updated_at = NOW()
                 WHERE id = 1",
            )
            .execute(target.inner())
            .await;
            let _ = sqlx::query(
                "UPDATE cvm_analytics.analytics_projection_target_status
                 SET ready = FALSE, last_error_code = 'snapshot_copy_failed', updated_at = NOW()",
            )
            .execute(target.inner())
            .await;
            return Err(error);
        }
        let current_source = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(
            "SELECT owner_kind, owner_epoch, serving_fence, generation, serving_ack_change_id
             FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(source_pool)
        .await;
        let (
            current_owner_kind,
            current_owner_epoch,
            current_serving_fence,
            current_generation,
            current_serving_ack_change_id,
        ) = match current_source {
            Ok(value) => value,
            Err(error) => {
                self.invalidate_long_term_projection_for_refresh()
                    .await
                    .context("failed to fail-closed after source freshness read failure")?;
                return Err(error.into());
            }
        };
        if current_owner_kind != source_fence.0
            || current_owner_epoch != source_fence.1
            || current_serving_fence != source_fence.2
            || current_generation != source_fence.3
            || current_serving_ack_change_id > snapshot.cursor_row_id
        {
            // The copy has already committed, so force the target into a fail-closed
            // state before returning the freshness error. This path must not depend on
            // the source-side fence transaction succeeding again.
            self.invalidate_long_term_projection_for_refresh()
                .await
                .context("failed to fail-closed stale PostgreSQL analytics projection")?;
            bail!("analytics source owner fence changed during PostgreSQL materialization");
        }
        Ok(LongTermProjectionMaterializationReport {
            row_count: snapshot.row_count,
            source_cursor_row_id: snapshot.cursor_row_id,
            digest: snapshot.digest,
        })
    }
}

async fn load_source_snapshot(source_pool: &Pool<Sqlite>) -> Result<SourceSnapshot> {
    let mut tx = source_pool.begin().await?;
    let (owner_kind, owner_epoch, serving_fence, generation, serving_ack_change_id):
        (String, i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT owner_kind, owner_epoch, serving_fence, generation, serving_ack_change_id FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&mut *tx)
        .await?;
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&mut *tx)
            .await?;
    anyhow::ensure!(
        status == "ready",
        "SQLite long-term projection is not ready"
    );
    let cursor_row_id = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(cursor_row_id, 0) FROM long_term_projection_state WHERE consumer = ?1",
    )
    .bind(PROJECTION_CONSUMER)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or_default();
    let daily = sqlx::query_as::<_, DailyRow>(
        "SELECT stats_date, dimension, series_key, display_name, reasoning_effort, calls,
                token_total, token_samples, cost_total, cost_samples, usage_time_ms,
                usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total,
                stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples,
                response_sum_ms, response_samples
         FROM long_term_usage_daily ORDER BY stats_date, dimension, series_key",
    )
    .fetch_all(&mut *tx)
    .await?;
    let hourly = sqlx::query_as::<_, HourlyRow>(
        "SELECT bucket_start_epoch, dimension, series_key, display_name, reasoning_effort, calls,
                token_total, token_samples, cost_total, cost_samples, usage_time_ms,
                usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total,
                stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples,
                response_sum_ms, response_samples
         FROM long_term_usage_hourly ORDER BY bucket_start_epoch, dimension, series_key",
    )
    .fetch_all(&mut *tx)
    .await?;
    let interval_state = sqlx::query_as::<_, IntervalStateRow>(
        "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms
         FROM long_term_projection_interval_state ORDER BY invocation_row_id",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut digest = DigestBuilder::new();
    for row in &daily {
        digest.daily(row);
    }
    for row in &hourly {
        digest.hourly(row);
    }
    Ok(SourceSnapshot {
        row_count: (daily.len() + hourly.len()) as i64,
        daily,
        hourly,
        interval_state,
        cursor_row_id,
        digest: digest.finish(),
        owner_kind,
        owner_epoch,
        serving_fence,
        generation,
        serving_ack_change_id,
    })
}

async fn copy_snapshot(target: &sqlx::PgPool, snapshot: &SourceSnapshot) -> Result<()> {
    let mut tx = target.begin().await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_daily_rollups")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_hourly_rollups")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_projection_interval_state")
        .execute(&mut *tx)
        .await?;
    for row in &snapshot.daily {
        sqlx::query(
            "INSERT INTO cvm_analytics.analytics_daily_rollups
            (stats_date, dimension, series_key, display_name, reasoning_effort, calls, token_total,
             token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples,
             wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms,
             output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms,
             response_samples)
            VALUES ($1::date,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)",
        )
        .bind(&row.stats_date)
        .bind(&row.dimension)
        .bind(&row.series_key)
        .bind(&row.display_name)
        .bind(&row.reasoning_effort)
        .bind(row.calls)
        .bind(row.token_total)
        .bind(row.token_samples)
        .bind(row.cost_total)
        .bind(row.cost_samples)
        .bind(row.usage_time_ms)
        .bind(row.usage_time_samples)
        .bind(row.wall_time_ms)
        .bind(row.wall_time_samples)
        .bind(row.output_tokens_total)
        .bind(row.stream_duration_ms)
        .bind(row.output_speed_samples)
        .bind(row.first_byte_sum_ms)
        .bind(row.first_byte_samples)
        .bind(row.response_sum_ms)
        .bind(row.response_samples)
        .execute(&mut *tx)
        .await?;
    }
    for row in &snapshot.hourly {
        anyhow::ensure!(
            DateTime::<Utc>::from_timestamp(row.bucket_start_epoch, 0).is_some(),
            "invalid SQLite hourly bucket timestamp"
        );
        sqlx::query("INSERT INTO cvm_analytics.analytics_hourly_rollups
            (bucket_start, dimension, series_key, display_name, reasoning_effort, calls, token_total,
             token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples,
             wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms,
             output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms,
             response_samples)
            VALUES (to_timestamp($1),$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)")
            .bind(row.bucket_start_epoch as f64).bind(&row.dimension).bind(&row.series_key).bind(&row.display_name)
            .bind(&row.reasoning_effort).bind(row.calls).bind(row.token_total)
            .bind(row.token_samples).bind(row.cost_total).bind(row.cost_samples)
            .bind(row.usage_time_ms).bind(row.usage_time_samples).bind(row.wall_time_ms)
            .bind(row.wall_time_samples).bind(row.output_tokens_total).bind(row.stream_duration_ms)
            .bind(row.output_speed_samples).bind(row.first_byte_sum_ms).bind(row.first_byte_samples)
            .bind(row.response_sum_ms).bind(row.response_samples)
            .execute(&mut *tx).await?;
    }
    for row in &snapshot.interval_state {
        sqlx::query(
            "INSERT INTO cvm_analytics.analytics_projection_interval_state
             (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(row.invocation_row_id)
        .bind(&row.model_series_key)
        .bind(&row.upstream_series_key)
        .bind(row.interval_start_ms)
        .bind(row.interval_end_ms)
        .execute(&mut *tx)
        .await?;
    }
    let daily_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cvm_analytics.analytics_daily_rollups")
            .fetch_one(&mut *tx)
            .await?;
    let hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cvm_analytics.analytics_hourly_rollups")
            .fetch_one(&mut *tx)
            .await?;
    anyhow::ensure!(
        daily_count + hourly_count == snapshot.row_count,
        "PostgreSQL projection row count mismatch"
    );
    let verified_digest = load_target_digest(&mut tx).await?;
    anyhow::ensure!(
        verified_digest == snapshot.digest,
        "PostgreSQL projection digest mismatch"
    );
    let digest_hex = snapshot
        .digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    sqlx::query(
        "UPDATE cvm_analytics.analytics_projection_state
        SET status = 'ready', processed_rows = $1, total_rows = $1,
            source_row_count = $1, source_digest = $2,
            verified_row_count = $1, verified_digest = $2,
            last_error_code = NULL, updated_at = NOW() WHERE id = 1",
    )
    .bind(snapshot.row_count)
    .bind(&digest_hex)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO cvm_analytics.analytics_projection_checkpoints
        (consumer, cursor_row_id, last_flush_at, last_error_code, generation)
        VALUES ($1,$2,NOW(),NULL,1)
        ON CONFLICT (consumer) DO UPDATE SET cursor_row_id = EXCLUDED.cursor_row_id,
            last_flush_at = EXCLUDED.last_flush_at, last_error_code = NULL,
            generation = analytics_projection_checkpoints.generation + 1, updated_at = NOW()",
    )
    .bind(PROJECTION_CONSUMER)
    .bind(snapshot.cursor_row_id)
    .execute(&mut *tx)
    .await?;
    // The snapshot proves only the two rollup targets copied above.  Keep the
    // interval/read-model and repair/bucket targets explicitly fenced until their
    // independent PG writers and parity checks are implemented.
    sqlx::query(
        "UPDATE cvm_analytics.analytics_projection_target_status
         SET ready = (target_kind IN ('daily_rollups', 'hourly_rollups')),
             generation = generation + 1,
             source_row_count = CASE WHEN target_kind IN ('daily_rollups', 'hourly_rollups') THEN $1 ELSE 0 END,
             source_digest = CASE WHEN target_kind IN ('daily_rollups', 'hourly_rollups') THEN $2 ELSE NULL END,
             last_error_code = CASE WHEN target_kind IN ('daily_rollups', 'hourly_rollups') THEN NULL ELSE 'target_not_materialized' END,
             updated_at = NOW()",
    )
    .bind(snapshot.row_count)
    .bind(&digest_hex)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn load_target_digest(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<[u8; 32]> {
    let daily = sqlx::query_as::<_, DailyRow>("SELECT stats_date::text, dimension, series_key,
        display_name, reasoning_effort, calls, token_total, token_samples, cost_total::double precision,
        cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples,
        output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms,
        first_byte_samples, response_sum_ms, response_samples FROM cvm_analytics.analytics_daily_rollups
        ORDER BY stats_date, dimension, series_key").fetch_all(&mut **tx).await?;
    let hourly = sqlx::query_as::<_, HourlyRow>(
        "SELECT EXTRACT(EPOCH FROM bucket_start)::bigint AS bucket_start_epoch,
        dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples,
        cost_total::double precision, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms,
        wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples,
        first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples
        FROM cvm_analytics.analytics_hourly_rollups ORDER BY bucket_start, dimension, series_key",
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut digest = DigestBuilder::new();
    for row in &daily {
        digest.daily(row);
    }
    for row in &hourly {
        digest.hourly(row);
    }
    Ok(digest.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_builder_separates_projection_kinds() {
        let daily = DailyRow {
            stats_date: "2026-11-11".to_string(),
            dimension: "overall".to_string(),
            series_key: "all".to_string(),
            display_name: "All".to_string(),
            reasoning_effort: String::new(),
            calls: 1,
            token_total: 2,
            token_samples: 1,
            cost_total: 0.5,
            cost_samples: 1,
            usage_time_ms: 0.0,
            usage_time_samples: 0,
            wall_time_ms: 0.0,
            wall_time_samples: 0,
            output_tokens_total: 0,
            stream_duration_ms: 0.0,
            output_speed_samples: 0,
            first_byte_sum_ms: 0.0,
            first_byte_samples: 0,
            response_sum_ms: 0.0,
            response_samples: 0,
        };
        let mut first = DigestBuilder::new();
        first.daily(&daily);
        let mut second = DigestBuilder::new();
        second.text("hourly");
        second.text(&daily.stats_date);
        assert_ne!(first.finish(), second.finish());
    }
}
