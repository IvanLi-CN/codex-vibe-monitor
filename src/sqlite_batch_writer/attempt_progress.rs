use super::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct BatchedAttemptProgress {
    pub(crate) attempt_id: i64,
    pub(crate) pending_status: &'static str,
    pub(crate) enqueue_sequence: u64,
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

fn merge_latest_optional_string(
    current: &mut Option<String>,
    incoming: Option<String>,
    current_sequence: u64,
    incoming_sequence: u64,
) {
    if incoming.is_some() && incoming_sequence >= current_sequence {
        *current = incoming;
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
        (Some(current), Some(incoming)) => {
            attempt_phase_rank(incoming) >= attempt_phase_rank(current)
        }
        _ => false,
    };
    if should_replace_phase {
        current.phase = incoming.phase;
    }
    merge_max_f64(&mut current.connect_latency_ms, incoming.connect_latency_ms);
    merge_max_f64(
        &mut current.first_byte_latency_ms,
        incoming.first_byte_latency_ms,
    );
    merge_latest_optional_string(
        &mut current.compact_support_status,
        incoming.compact_support_status,
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.compact_support_reason,
        incoming.compact_support_reason,
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.request_model,
        incoming.request_model,
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.upstream_request_model,
        incoming.upstream_request_model,
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.model_mapping_pattern,
        incoming.model_mapping_pattern,
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.request_summary_json,
        incoming.request_summary_json,
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.upstream_request_compression_algorithm,
        incoming.upstream_request_compression_algorithm,
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.upstream_request_compression_mode,
        incoming.upstream_request_compression_mode,
        current_sequence,
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
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.response_raw_codec,
        incoming.response_raw_codec,
        current_sequence,
        incoming_sequence,
    );
    merge_max_i64(&mut current.response_raw_size, incoming.response_raw_size);
    if incoming.response_raw_truncated.is_some() && incoming_sequence >= current_sequence {
        current.response_raw_truncated = incoming.response_raw_truncated;
    }
    merge_latest_optional_string(
        &mut current.response_raw_truncated_reason,
        incoming.response_raw_truncated_reason,
        current_sequence,
        incoming_sequence,
    );
    merge_latest_optional_string(
        &mut current.response_content_encoding,
        incoming.response_content_encoding,
        current_sequence,
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
                WHEN status = ?3 AND finished_at IS NULL THEN COALESCE(?2, phase)
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
