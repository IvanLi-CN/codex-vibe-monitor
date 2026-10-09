use super::*;

impl SqliteBatchWriter {
    pub(crate) fn enqueue_attempt_progress_reliably(
        &self,
        progress: BatchedAttemptProgress,
    ) -> bool {
        self.accounting.attempt_progress_enqueued();

        #[cfg(test)]
        if let Some(buffered_writes) = &self.buffered_writes {
            let _attempt_progress_send_guard = self
                .attempt_progress_send_gate
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let write = self.stamp_attempt_progress(SqliteBatchWrite::AttemptProgress(progress));
            let estimated_bytes = write.estimated_memory_bytes();
            match buffered_writes.lock() {
                Ok(mut guard) => {
                    guard.push(write);
                    self.accounting.enqueue(estimated_bytes);
                    return true;
                }
                Err(err) => {
                    self.accounting.attempt_progress_dropped();
                    self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        error = %err,
                        dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                        "sqlite batch writer test buffer poisoned; dropped reliable attempt progress"
                    );
                    return false;
                }
            }
        }

        let _attempt_progress_send_guard = self
            .attempt_progress_send_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let write = self.stamp_attempt_progress(SqliteBatchWrite::AttemptProgress(progress));
        let estimated_bytes = write.estimated_memory_bytes();
        self.accounting.enqueue(estimated_bytes);
        match self.write_sender.try_send(write) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(write)) => {
                self.accounting.attempt_progress_deferred(1);
                if self.admit_reliable_attempt_progress_overflow(write) {
                    true
                } else {
                    self.accounting.rollback_enqueue(estimated_bytes);
                    self.accounting.attempt_progress_dropped();
                    self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        queue_depth = self.accounting.snapshot().pending_depth,
                        dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                        "reliable attempt progress overflow is full; dropped derived progress"
                    );
                    false
                }
            }
            Err(mpsc::error::TrySendError::Closed(_write)) => {
                self.accounting.rollback_enqueue(estimated_bytes);
                self.accounting.attempt_progress_dropped();
                self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                warn!(
                    dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                    "sqlite batch writer closed; dropped reliable attempt progress"
                );
                false
            }
        }
    }

    fn admit_reliable_attempt_progress_overflow(&self, write: SqliteBatchWrite) -> bool {
        let SqliteBatchWrite::AttemptProgress(progress) = write else {
            return false;
        };
        let Ok(mut overflow) = self.reliable_attempt_progress_overflow.lock() else {
            return false;
        };
        let mut candidate = overflow.clone();
        candidate.push(SqliteBatchWrite::AttemptProgress(progress.clone()));
        if candidate.logical_rows() > SQLITE_RELIABLE_ATTEMPT_PROGRESS_OVERFLOW_MAX_ROWS
            || candidate.estimated_memory_bytes() > SQLITE_BATCH_MAX_BYTES
        {
            return false;
        }
        overflow.push_accounted(
            SqliteBatchWrite::AttemptProgress(progress),
            &self.accounting,
        );
        self.reliable_attempt_progress_notify.notify_one();
        true
    }
}

pub(super) fn drain_reliable_attempt_progress_overflow(
    overflow: &Arc<std::sync::Mutex<PendingBatch>>,
    pending: &mut PendingBatch,
    accounting: &PendingQueueAccounting,
    max_rows: usize,
) -> usize {
    if max_rows == 0 {
        return 0;
    }
    let overflow_batch = {
        let Ok(mut overflow) = overflow.lock() else {
            return 0;
        };
        if overflow.is_empty() {
            return 0;
        }
        overflow.take_p2_chunk(max_rows, SQLITE_BATCH_MAX_BYTES)
    };
    if overflow_batch.is_empty() {
        return 0;
    }
    let overflow_rows = overflow_batch.logical_rows();
    let overflow_bytes = overflow_batch.estimated_memory_bytes();
    let pending_rows = pending.logical_rows();
    let pending_bytes = pending.estimated_memory_bytes();
    pending.merge_p2(overflow_batch);
    accounting.replace_batch(
        pending_rows.saturating_add(overflow_rows),
        pending.logical_rows(),
        pending_bytes.saturating_add(overflow_bytes),
        pending.estimated_memory_bytes(),
    );
    overflow_rows
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AttemptProgressFieldSequences {
    pub(crate) phase: u64,
    pub(crate) compact_support_status: u64,
    pub(crate) compact_support_reason: u64,
    pub(crate) request_model: u64,
    pub(crate) upstream_request_model: u64,
    pub(crate) model_mapping_pattern: u64,
    pub(crate) request_summary_json: u64,
    pub(crate) upstream_request_compression_algorithm: u64,
    pub(crate) upstream_request_compression_mode: u64,
    pub(crate) response_raw_path: u64,
    pub(crate) response_raw_codec: u64,
    pub(crate) response_raw_truncated: u64,
    pub(crate) response_raw_truncated_reason: u64,
    pub(crate) response_content_encoding: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BatchedAttemptProgress {
    pub(crate) attempt_id: i64,
    pub(crate) pending_status: &'static str,
    pub(crate) enqueue_sequence: u64,
    pub(crate) field_sequences: AttemptProgressFieldSequences,
    pub(crate) phase: Option<String>,
    pub(crate) connect_latency_ms: Option<f64>,
    pub(crate) first_byte_latency_ms: Option<f64>,
    pub(crate) compact_support_status: Option<String>,
    pub(crate) compact_support_reason: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) upstream_request_model: Option<String>,
    pub(crate) model_mapping_pattern: Option<String>,
    pub(crate) request_summary_json: Option<String>,
    pub(crate) upstream_request_compression_algorithm: Option<String>,
    pub(crate) upstream_request_compression_mode: Option<String>,
    pub(crate) upstream_request_logical_body_bytes: Option<i64>,
    pub(crate) upstream_request_transmitted_body_bytes: Option<i64>,
    pub(crate) upstream_request_header_bytes_approx: Option<i64>,
    pub(crate) upstream_response_body_bytes: Option<i64>,
    pub(crate) upstream_response_header_bytes_approx: Option<i64>,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) response_raw_codec: Option<String>,
    pub(crate) response_raw_size: Option<i64>,
    pub(crate) response_raw_truncated: Option<bool>,
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
}

impl BatchedAttemptProgress {
    pub(crate) fn stamp_sequence(&mut self, sequence: u64) {
        self.enqueue_sequence = sequence;
        if self.phase.is_some() {
            self.field_sequences.phase = sequence;
        }
        if self.compact_support_status.is_some() {
            self.field_sequences.compact_support_status = sequence;
        }
        if self.compact_support_reason.is_some() {
            self.field_sequences.compact_support_reason = sequence;
        }
        if self.request_model.is_some() {
            self.field_sequences.request_model = sequence;
        }
        if self.upstream_request_model.is_some() {
            self.field_sequences.upstream_request_model = sequence;
        }
        if self.model_mapping_pattern.is_some() {
            self.field_sequences.model_mapping_pattern = sequence;
        }
        if self.request_summary_json.is_some() {
            self.field_sequences.request_summary_json = sequence;
        }
        if self.upstream_request_compression_algorithm.is_some() {
            self.field_sequences.upstream_request_compression_algorithm = sequence;
        }
        if self.upstream_request_compression_mode.is_some() {
            self.field_sequences.upstream_request_compression_mode = sequence;
        }
        if self.response_raw_path.is_some() {
            self.field_sequences.response_raw_path = sequence;
        }
        if self.response_raw_codec.is_some() {
            self.field_sequences.response_raw_codec = sequence;
        }
        if self.response_raw_truncated.is_some() {
            self.field_sequences.response_raw_truncated = sequence;
        }
        if self.response_raw_truncated.is_some() || self.response_raw_truncated_reason.is_some() {
            self.field_sequences.response_raw_truncated_reason = sequence;
        }
        if self.response_content_encoding.is_some() {
            self.field_sequences.response_content_encoding = sequence;
        }
    }
}

pub(crate) fn estimated_memory_bytes(progress: &BatchedAttemptProgress) -> usize {
    std::mem::size_of::<BatchedAttemptProgress>()
        .saturating_add(estimated_option_string_bytes(&progress.phase))
        .saturating_add(estimated_option_string_bytes(
            &progress.compact_support_status,
        ))
        .saturating_add(estimated_option_string_bytes(
            &progress.compact_support_reason,
        ))
        .saturating_add(estimated_option_string_bytes(&progress.request_model))
        .saturating_add(estimated_option_string_bytes(
            &progress.upstream_request_model,
        ))
        .saturating_add(estimated_option_string_bytes(
            &progress.model_mapping_pattern,
        ))
        .saturating_add(estimated_option_string_bytes(
            &progress.request_summary_json,
        ))
        .saturating_add(estimated_option_string_bytes(
            &progress.upstream_request_compression_algorithm,
        ))
        .saturating_add(estimated_option_string_bytes(
            &progress.upstream_request_compression_mode,
        ))
        .saturating_add(estimated_option_string_bytes(&progress.response_raw_path))
        .saturating_add(estimated_option_string_bytes(&progress.response_raw_codec))
        .saturating_add(estimated_option_string_bytes(
            &progress.response_raw_truncated_reason,
        ))
        .saturating_add(estimated_option_string_bytes(
            &progress.response_content_encoding,
        ))
}

fn attempt_phase_rank(phase: &str) -> u8 {
    match phase {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING => 0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST => 1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE => 2,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE => 3,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
        | POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED => 4,
        _ => 1,
    }
}

fn effective_field_sequence(field_sequence: u64, record_sequence: u64) -> u64 {
    if field_sequence == 0 {
        record_sequence
    } else {
        field_sequence
    }
}

fn merge_latest_optional_string(
    current: &mut Option<String>,
    incoming: Option<String>,
    current_field_sequence: &mut u64,
    current_record_sequence: u64,
    incoming_field_sequence: u64,
    incoming_record_sequence: u64,
) {
    let Some(incoming) = incoming else {
        return;
    };
    let current_sequence =
        effective_field_sequence(*current_field_sequence, current_record_sequence);
    let incoming_sequence =
        effective_field_sequence(incoming_field_sequence, incoming_record_sequence);
    if (current.is_none() && *current_field_sequence == 0) || incoming_sequence >= current_sequence
    {
        *current = Some(incoming);
        *current_field_sequence = incoming_sequence;
    }
}

fn merge_max_f64(current: &mut Option<f64>, incoming: Option<f64>) {
    if let Some(incoming) = incoming
        && current.is_none_or(|current| current < incoming)
    {
        *current = Some(incoming);
    }
}

fn merge_max_i64(current: &mut Option<i64>, incoming: Option<i64>) {
    if let Some(incoming) = incoming
        && current.is_none_or(|current| current < incoming)
    {
        *current = Some(incoming);
    }
}

pub(crate) fn merge(current: &mut BatchedAttemptProgress, incoming: BatchedAttemptProgress) {
    let current_sequence = current.enqueue_sequence;
    let incoming_sequence = incoming.enqueue_sequence;
    let should_replace_phase = match (current.phase.as_deref(), incoming.phase.as_deref()) {
        (None, Some(_)) => true,
        (Some(current_phase), Some(incoming_phase)) => {
            let incoming_rank = attempt_phase_rank(incoming_phase);
            let current_rank = attempt_phase_rank(current_phase);
            incoming_rank > current_rank
                || (incoming_rank == current_rank
                    && effective_field_sequence(incoming.field_sequences.phase, incoming_sequence)
                        >= effective_field_sequence(
                            current.field_sequences.phase,
                            current_sequence,
                        ))
        }
        _ => false,
    };
    if should_replace_phase {
        current.phase = incoming.phase;
        current.field_sequences.phase =
            effective_field_sequence(incoming.field_sequences.phase, incoming_sequence);
    }
    merge_max_f64(&mut current.connect_latency_ms, incoming.connect_latency_ms);
    merge_max_f64(
        &mut current.first_byte_latency_ms,
        incoming.first_byte_latency_ms,
    );
    merge_latest_optional_string(
        &mut current.compact_support_status,
        incoming.compact_support_status,
        &mut current.field_sequences.compact_support_status,
        current_sequence,
        incoming.field_sequences.compact_support_status,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.compact_support_reason,
        incoming.compact_support_reason,
        &mut current.field_sequences.compact_support_reason,
        current_sequence,
        incoming.field_sequences.compact_support_reason,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.request_model,
        incoming.request_model,
        &mut current.field_sequences.request_model,
        current_sequence,
        incoming.field_sequences.request_model,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.upstream_request_model,
        incoming.upstream_request_model,
        &mut current.field_sequences.upstream_request_model,
        current_sequence,
        incoming.field_sequences.upstream_request_model,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.model_mapping_pattern,
        incoming.model_mapping_pattern,
        &mut current.field_sequences.model_mapping_pattern,
        current_sequence,
        incoming.field_sequences.model_mapping_pattern,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.request_summary_json,
        incoming.request_summary_json,
        &mut current.field_sequences.request_summary_json,
        current_sequence,
        incoming.field_sequences.request_summary_json,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.upstream_request_compression_algorithm,
        incoming.upstream_request_compression_algorithm,
        &mut current
            .field_sequences
            .upstream_request_compression_algorithm,
        current_sequence,
        incoming
            .field_sequences
            .upstream_request_compression_algorithm,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.upstream_request_compression_mode,
        incoming.upstream_request_compression_mode,
        &mut current.field_sequences.upstream_request_compression_mode,
        current_sequence,
        incoming.field_sequences.upstream_request_compression_mode,
        incoming_sequence,
    );
    merge_max_i64(
        &mut current.upstream_request_logical_body_bytes,
        incoming.upstream_request_logical_body_bytes,
    );
    merge_max_i64(
        &mut current.upstream_request_transmitted_body_bytes,
        incoming.upstream_request_transmitted_body_bytes,
    );
    merge_max_i64(
        &mut current.upstream_request_header_bytes_approx,
        incoming.upstream_request_header_bytes_approx,
    );
    merge_max_i64(
        &mut current.upstream_response_body_bytes,
        incoming.upstream_response_body_bytes,
    );
    merge_max_i64(
        &mut current.upstream_response_header_bytes_approx,
        incoming.upstream_response_header_bytes_approx,
    );
    merge_latest_optional_string(
        &mut current.response_raw_path,
        incoming.response_raw_path,
        &mut current.field_sequences.response_raw_path,
        current_sequence,
        incoming.field_sequences.response_raw_path,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.response_raw_codec,
        incoming.response_raw_codec,
        &mut current.field_sequences.response_raw_codec,
        current_sequence,
        incoming.field_sequences.response_raw_codec,
        incoming_sequence,
    );
    merge_max_i64(&mut current.response_raw_size, incoming.response_raw_size);
    if incoming.response_raw_truncated.is_some() {
        let current_field_sequence = effective_field_sequence(
            current.field_sequences.response_raw_truncated,
            current_sequence,
        );
        let incoming_field_sequence = effective_field_sequence(
            incoming.field_sequences.response_raw_truncated,
            incoming_sequence,
        );
        if current.response_raw_truncated.is_none()
            || incoming_field_sequence >= current_field_sequence
        {
            current.response_raw_truncated = incoming.response_raw_truncated;
            current.field_sequences.response_raw_truncated = incoming_field_sequence;
        }
    }
    if current.response_raw_truncated.is_some() {
        current.field_sequences.response_raw_truncated_reason = current
            .field_sequences
            .response_raw_truncated_reason
            .max(effective_field_sequence(
                current.field_sequences.response_raw_truncated,
                current_sequence,
            ));
    }
    let incoming_raw_truncated_reason_sequence = effective_field_sequence(
        incoming.field_sequences.response_raw_truncated_reason,
        incoming
            .field_sequences
            .response_raw_truncated
            .max(incoming_sequence),
    );
    merge_latest_optional_string(
        &mut current.response_raw_truncated_reason,
        incoming.response_raw_truncated_reason,
        &mut current.field_sequences.response_raw_truncated_reason,
        current_sequence,
        incoming_raw_truncated_reason_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.response_content_encoding,
        incoming.response_content_encoding,
        &mut current.field_sequences.response_content_encoding,
        current_sequence,
        incoming.field_sequences.response_content_encoding,
        incoming_sequence,
    );
    current.enqueue_sequence = current_sequence.max(incoming_sequence);
}

pub(crate) async fn persist(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    progress: &BatchedAttemptProgress,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            phase = CASE
                WHEN status = ?3
                    AND finished_at IS NULL
                    AND (
                        ?2 IS NULL
                        OR phase IS NULL
                        OR CASE LOWER(TRIM(?2))
                            WHEN 'connecting' THEN 0
                            WHEN 'sending_request' THEN 1
                            WHEN 'waiting_first_byte' THEN 2
                            WHEN 'streaming_response' THEN 3
                            WHEN 'completed' THEN 4
                            WHEN 'failed' THEN 4
                            ELSE 1
                        END >= CASE LOWER(TRIM(phase))
                            WHEN 'connecting' THEN 0
                            WHEN 'sending_request' THEN 1
                            WHEN 'waiting_first_byte' THEN 2
                            WHEN 'streaming_response' THEN 3
                            WHEN 'completed' THEN 4
                            WHEN 'failed' THEN 4
                            ELSE 1
                        END
                    ) THEN COALESCE(?2, phase)
                ELSE phase
            END,
            connect_latency_ms = CASE
                WHEN status = ?3 AND finished_at IS NULL AND ?4 IS NOT NULL
                    AND (connect_latency_ms IS NULL OR connect_latency_ms < ?4) THEN ?4
                ELSE connect_latency_ms
            END,
            first_byte_latency_ms = CASE
                WHEN status = ?3 AND finished_at IS NULL AND ?5 IS NOT NULL
                    AND (first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?5) THEN ?5
                ELSE first_byte_latency_ms
            END,
            compact_support_status = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?6, compact_support_status)
                ELSE compact_support_status
            END,
            compact_support_reason = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?7, compact_support_reason)
                ELSE compact_support_reason
            END,
            request_model = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?8, request_model)
                ELSE request_model
            END,
            upstream_request_model = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?9, upstream_request_model)
                ELSE upstream_request_model
            END,
            model_mapping_pattern = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?10, model_mapping_pattern)
                ELSE model_mapping_pattern
            END,
            request_summary_json = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?11, request_summary_json)
                ELSE request_summary_json
            END,
            upstream_request_compression_algorithm = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?12, upstream_request_compression_algorithm)
                ELSE upstream_request_compression_algorithm
            END,
            upstream_request_compression_mode = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?13, upstream_request_compression_mode)
                ELSE upstream_request_compression_mode
            END,
            upstream_request_logical_body_bytes = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?14, upstream_request_logical_body_bytes)
                ELSE upstream_request_logical_body_bytes
            END,
            upstream_request_transmitted_body_bytes = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?15, upstream_request_transmitted_body_bytes)
                ELSE upstream_request_transmitted_body_bytes
            END,
            upstream_request_header_bytes_approx = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?16, upstream_request_header_bytes_approx)
                ELSE upstream_request_header_bytes_approx
            END,
            upstream_response_body_bytes = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?17, upstream_response_body_bytes)
                ELSE upstream_response_body_bytes
            END,
            upstream_response_header_bytes_approx = CASE
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?18, upstream_response_header_bytes_approx)
                ELSE upstream_response_header_bytes_approx
            END,
            response_raw_path = COALESCE(?19, response_raw_path),
            response_raw_codec = COALESCE(?20, response_raw_codec),
            response_raw_size = COALESCE(?21, response_raw_size),
            response_raw_truncated = COALESCE(?22, response_raw_truncated),
            response_raw_truncated_reason = COALESCE(?23, response_raw_truncated_reason),
            response_content_encoding = COALESCE(?24, response_content_encoding)
        WHERE id = ?1
          AND (
                (status = ?3 AND finished_at IS NULL)
                OR ?19 IS NOT NULL
                OR ?20 IS NOT NULL
                OR ?21 IS NOT NULL
                OR ?22 IS NOT NULL
                OR ?23 IS NOT NULL
                OR ?24 IS NOT NULL
              )
        "#,
    )
    .bind(progress.attempt_id)
    .bind(progress.phase.as_deref())
    .bind(progress.pending_status)
    .bind(progress.connect_latency_ms)
    .bind(progress.first_byte_latency_ms)
    .bind(progress.compact_support_status.as_deref())
    .bind(progress.compact_support_reason.as_deref())
    .bind(progress.request_model.as_deref())
    .bind(progress.upstream_request_model.as_deref())
    .bind(progress.model_mapping_pattern.as_deref())
    .bind(progress.request_summary_json.as_deref())
    .bind(progress.upstream_request_compression_algorithm.as_deref())
    .bind(progress.upstream_request_compression_mode.as_deref())
    .bind(progress.upstream_request_logical_body_bytes)
    .bind(progress.upstream_request_transmitted_body_bytes)
    .bind(progress.upstream_request_header_bytes_approx)
    .bind(progress.upstream_response_body_bytes)
    .bind(progress.upstream_response_header_bytes_approx)
    .bind(progress.response_raw_path.as_deref())
    .bind(progress.response_raw_codec.as_deref())
    .bind(progress.response_raw_size)
    .bind(progress.response_raw_truncated.map(i64::from))
    .bind(progress.response_raw_truncated_reason.as_deref())
    .bind(progress.response_content_encoding.as_deref())
    .execute(tx.as_mut())
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect sqlite memory pool");
        ensure_schema(&pool).await.expect("ensure schema");
        pool
    }

    async fn pending_attempt(pool: &SqlitePool, invoke_id: &str) -> PendingPoolAttemptRecord {
        let trace = PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: "2026-07-01 10:00:00".to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some(format!("{invoke_id}-sticky")),
            requester_ip: Some("192.168.31.6".to_string()),
            upstream_base_url_host: None,
            request_model: None,
        };
        let pending = begin_pool_upstream_request_attempt(
            pool,
            &trace,
            101,
            "route-primary",
            1,
            1,
            1,
            "2026-07-01 10:00:00",
        )
        .await;
        assert!(pending.attempt_id.is_some());
        pending
    }

    #[tokio::test]
    async fn coalesces_attempt_progress_and_preserves_latest_metadata() {
        let pool = test_pool().await;
        let attempt_id = pending_attempt(&pool, "batch-progress-coalesce")
            .await
            .attempt_id
            .expect("attempt id");
        sqlx::query(
            "CREATE TABLE attempt_progress_write_counter (writes INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .expect("create attempt progress counter");
        sqlx::query("INSERT INTO attempt_progress_write_counter DEFAULT VALUES")
            .execute(&pool)
            .await
            .expect("seed attempt progress counter");
        sqlx::query(
            r#"CREATE TRIGGER attempt_progress_write_counter_trigger
               AFTER UPDATE OF phase, request_model, upstream_request_model, model_mapping_pattern,
                   request_summary_json, upstream_request_compression_algorithm,
                   upstream_request_compression_mode, upstream_request_logical_body_bytes,
                   upstream_request_transmitted_body_bytes, upstream_request_header_bytes_approx,
                   upstream_response_body_bytes, upstream_response_header_bytes_approx,
                   response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated,
                   response_raw_truncated_reason, response_content_encoding
               ON pool_upstream_request_attempts
               BEGIN
                   UPDATE attempt_progress_write_counter SET writes = writes + 1;
               END"#,
        )
        .execute(&pool)
        .await
        .expect("create attempt progress write counter trigger");

        let progress =
            |phase: &str, upstream_model: &str, compression: &str| BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                enqueue_sequence: 0,
                field_sequences: Default::default(),
                phase: Some(phase.to_string()),
                connect_latency_ms: Some(18.0),
                first_byte_latency_ms: Some(33.0),
                compact_support_status: Some("supported".to_string()),
                compact_support_reason: Some("cached_probe".to_string()),
                request_model: Some("requested-model".to_string()),
                upstream_request_model: Some(upstream_model.to_string()),
                model_mapping_pattern: Some("map-b".to_string()),
                request_summary_json: Some(r#"{"rewrite":"b"}"#.to_string()),
                upstream_request_compression_algorithm: Some(compression.to_string()),
                upstream_request_compression_mode: Some("forced".to_string()),
                upstream_request_logical_body_bytes: Some(140),
                upstream_request_transmitted_body_bytes: Some(120),
                upstream_request_header_bytes_approx: Some(16),
                upstream_response_body_bytes: Some(320),
                upstream_response_header_bytes_approx: Some(24),
                response_raw_path: Some("captures/attempt.raw".to_string()),
                response_raw_codec: Some("zstd".to_string()),
                response_raw_size: Some(320),
                response_raw_truncated: Some(true),
                response_raw_truncated_reason: Some("limit".to_string()),
                response_content_encoding: Some("gzip".to_string()),
            };
        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![
                SqliteBatchWrite::AttemptProgress(progress(
                    POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
                    "upstream-model-a",
                    "gzip",
                )),
                SqliteBatchWrite::AttemptProgress(progress(
                    POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
                    "upstream-model-b",
                    "br",
                )),
            ],
        )
        .await;

        let row = sqlx::query_as::<_, (Option<String>, Option<String>, Option<f64>, Option<f64>)>(
            concat!(
                "SELECT phase, request_model, connect_latency_ms, first_byte_latency_ms ",
                "FROM pool_upstream_request_attempts WHERE id = ?1"
            ),
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load coalesced attempt");
        assert_eq!(
            row.0.as_deref(),
            Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
        );
        assert_eq!(row.1.as_deref(), Some("requested-model"));
        assert_eq!(row.2, Some(18.0));
        assert_eq!(row.3, Some(33.0));
        let metadata = sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(concat!(
            "SELECT upstream_request_model, model_mapping_pattern, request_summary_json, ",
            "upstream_request_compression_algorithm, upstream_request_compression_mode ",
            "FROM pool_upstream_request_attempts WHERE id = ?1"
        ))
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load coalesced metadata");
        assert_eq!(metadata.0.as_deref(), Some("upstream-model-b"));
        assert_eq!(metadata.1.as_deref(), Some("map-b"));
        assert_eq!(metadata.2.as_deref(), Some(r#"{"rewrite":"b"}"#));
        assert_eq!(metadata.3.as_deref(), Some("br"));
        assert_eq!(metadata.4.as_deref(), Some("forced"));
        let bytes = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>)>(
            concat!(
                "SELECT upstream_request_logical_body_bytes, upstream_request_transmitted_body_bytes, ",
                "upstream_request_header_bytes_approx, upstream_response_body_bytes, ",
                "upstream_response_header_bytes_approx FROM pool_upstream_request_attempts WHERE id = ?1"
            ),
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load coalesced byte metadata");
        assert_eq!(bytes, (Some(140), Some(120), Some(16), Some(320), Some(24)));
        let raw = sqlx::query_as::<_, (Option<String>, Option<String>, Option<i64>, Option<i64>, Option<String>, Option<String>)>(
            concat!(
                "SELECT response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated, ",
                "response_raw_truncated_reason, response_content_encoding ",
                "FROM pool_upstream_request_attempts WHERE id = ?1"
            ),
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load coalesced raw metadata");
        assert_eq!(
            raw,
            (
                Some("captures/attempt.raw".to_string()),
                Some("zstd".to_string()),
                Some(320),
                Some(1),
                Some("limit".to_string()),
                Some("gzip".to_string())
            )
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT writes FROM attempt_progress_write_counter")
                .fetch_one(&pool)
                .await
                .expect("load attempt progress write count"),
            1
        );
    }

    #[tokio::test]
    async fn stale_attempt_progress_does_not_overwrite_terminal_finalize() {
        let pool = test_pool().await;
        let mut pending = pending_attempt(&pool, "batch-progress-terminal-cover").await;
        let attempt_id = pending.attempt_id.expect("attempt id");
        pending.request_model = Some("terminal-request-model".to_string());
        pending.upstream_request_model = Some("terminal-upstream-model".to_string());
        pending.model_mapping_pattern = Some("terminal-map".to_string());
        pending.request_summary_json = Some(r#"{"rewrite":"terminal"}"#.to_string());
        finalize_pool_upstream_request_attempt(
            &pool,
            &pending,
            "2026-07-01 10:00:05",
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            Some(StatusCode::OK),
            None,
            None,
            None,
            None,
            Some(42.0),
            Some(16.0),
            Some(188.0),
            Some("req_terminal"),
            None,
            None,
        )
        .await
        .expect("finalize attempt synchronously");
        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE.to_string()),
                connect_latency_ms: Some(99.0),
                first_byte_latency_ms: Some(99.0),
                compact_support_status: Some("stale".to_string()),
                compact_support_reason: Some("should_not_apply".to_string()),
                request_model: Some("stale-request-model".to_string()),
                upstream_request_model: Some("stale-upstream-model".to_string()),
                model_mapping_pattern: Some("stale-map".to_string()),
                request_summary_json: Some(r#"{"rewrite":"stale"}"#.to_string()),
                upstream_request_compression_algorithm: Some("stale-compression".to_string()),
                upstream_request_compression_mode: Some("stale-mode".to_string()),
                ..Default::default()
            })],
        )
        .await;
        let row = sqlx::query_as::<_, (String, Option<String>, Option<f64>, Option<String>, Option<String>, Option<String>, Option<String>)>(
            "SELECT status, phase, connect_latency_ms, request_model, upstream_request_model, model_mapping_pattern, request_summary_json FROM pool_upstream_request_attempts WHERE id = ?1",
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load finalized attempt");
        assert_eq!(row.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
        assert_eq!(
            row.1.as_deref(),
            Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
        );
        assert_eq!(row.2, Some(42.0));
        assert_eq!(row.3.as_deref(), Some("terminal-request-model"));
        assert_eq!(row.4.as_deref(), Some("terminal-upstream-model"));
        assert_eq!(row.5.as_deref(), Some("terminal-map"));
        assert_eq!(row.6.as_deref(), Some(r#"{"rewrite":"terminal"}"#));
    }

    #[test]
    fn sparse_progress_fields_merge_by_their_own_sequence() {
        let mut merged = BatchedAttemptProgress {
            attempt_id: 7,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            enqueue_sequence: 10,
            field_sequences: AttemptProgressFieldSequences {
                phase: 10,
                request_model: 10,
                ..Default::default()
            },
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST.to_string()),
            request_model: Some("old-request-model".to_string()),
            ..Default::default()
        };
        let newer_phase = BatchedAttemptProgress {
            attempt_id: 7,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            enqueue_sequence: 30,
            field_sequences: AttemptProgressFieldSequences {
                phase: 30,
                ..Default::default()
            },
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string()),
            ..Default::default()
        };
        let retained_request_model = BatchedAttemptProgress {
            attempt_id: 7,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            enqueue_sequence: 20,
            field_sequences: AttemptProgressFieldSequences {
                request_model: 20,
                ..Default::default()
            },
            request_model: Some("new-request-model".to_string()),
            ..Default::default()
        };

        merge(&mut merged, newer_phase);
        merge(&mut merged, retained_request_model);

        assert_eq!(
            merged.phase.as_deref(),
            Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
        );
        assert_eq!(merged.request_model.as_deref(), Some("new-request-model"));
    }

    #[tokio::test]
    async fn stale_phase_from_a_later_batch_cannot_regress_persisted_phase() {
        let pool = test_pool().await;
        let attempt_id = pending_attempt(&pool, "batch-progress-phase-rank")
            .await
            .attempt_id
            .expect("attempt id");

        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                enqueue_sequence: 20,
                phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string()),
                ..Default::default()
            })],
        )
        .await;
        SqliteBatchWriter::flush_for_test(
            &pool,
            vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                enqueue_sequence: 10,
                phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE.to_string()),
                ..Default::default()
            })],
        )
        .await;

        let phase = sqlx::query_scalar::<_, Option<String>>(
            "SELECT phase FROM pool_upstream_request_attempts WHERE id = ?1",
        )
        .bind(attempt_id)
        .fetch_one(&pool)
        .await
        .expect("load persisted phase");
        assert_eq!(
            phase.as_deref(),
            Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
        );
    }

    #[test]
    fn retained_older_progress_cannot_overwrite_newer_metadata() {
        let mut newer = BatchedAttemptProgress {
            attempt_id: 7,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            enqueue_sequence: 20,
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string()),
            request_model: Some("new-request-model".to_string()),
            upstream_request_model: Some("new-upstream-model".to_string()),
            model_mapping_pattern: Some("new-map".to_string()),
            request_summary_json: Some(r#"{"rewrite":"new"}"#.to_string()),
            upstream_request_compression_algorithm: Some("br".to_string()),
            response_raw_path: Some("captures/new.raw".to_string()),
            response_raw_codec: Some("zstd".to_string()),
            response_raw_size: Some(200),
            response_raw_truncated: Some(false),
            response_raw_truncated_reason: None,
            response_content_encoding: Some("br".to_string()),
            ..Default::default()
        };
        let older = BatchedAttemptProgress {
            attempt_id: 7,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            enqueue_sequence: 10,
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE.to_string()),
            request_model: Some("old-request-model".to_string()),
            upstream_request_model: Some("old-upstream-model".to_string()),
            model_mapping_pattern: Some("old-map".to_string()),
            request_summary_json: Some(r#"{"rewrite":"old"}"#.to_string()),
            upstream_request_compression_algorithm: Some("gzip".to_string()),
            response_raw_path: Some("captures/old.raw".to_string()),
            response_raw_codec: Some("gzip".to_string()),
            response_raw_size: Some(100),
            response_raw_truncated: Some(true),
            response_raw_truncated_reason: Some("old-limit".to_string()),
            response_content_encoding: Some("gzip".to_string()),
            ..Default::default()
        };

        merge(&mut newer, older);

        assert_eq!(newer.enqueue_sequence, 20);
        assert_eq!(
            newer.phase.as_deref(),
            Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
        );
        assert_eq!(newer.request_model.as_deref(), Some("new-request-model"));
        assert_eq!(
            newer.upstream_request_model.as_deref(),
            Some("new-upstream-model")
        );
        assert_eq!(newer.model_mapping_pattern.as_deref(), Some("new-map"));
        assert_eq!(
            newer.request_summary_json.as_deref(),
            Some(r#"{"rewrite":"new"}"#)
        );
        assert_eq!(
            newer.upstream_request_compression_algorithm.as_deref(),
            Some("br")
        );
        assert_eq!(newer.response_raw_path.as_deref(), Some("captures/new.raw"));
        assert_eq!(newer.response_raw_codec.as_deref(), Some("zstd"));
        assert_eq!(newer.response_raw_size, Some(200));
        assert_eq!(newer.response_raw_truncated, Some(false));
        assert_eq!(newer.response_raw_truncated_reason, None);
        assert_eq!(newer.response_content_encoding.as_deref(), Some("br"));
    }
}
