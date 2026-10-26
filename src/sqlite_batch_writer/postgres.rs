use super::*;
use sqlx::PgConnection;

struct PostgresRawPayloadLink<'a> {
    raw_path: &'a str,
    storage_codec: &'a str,
    logical_size_bytes: i64,
    raw_role: &'a str,
}

struct PostgresRawPayloadMeta<'a> {
    raw_path: Option<&'a str>,
    storage_codec: Option<&'a str>,
    logical_size_bytes: Option<i64>,
}

async fn replace_postgres_raw_payload_links(
    tx: &mut PgConnection,
    owner_kind: &str,
    owner_id: i64,
    links: &[PostgresRawPayloadLink<'_>],
) -> Result<()> {
    let retained_paths = links
        .iter()
        .map(|link| link.raw_path)
        .collect::<HashSet<_>>();
    let released_paths = sqlx::query_scalar::<_, String>(
        "DELETE FROM proxy_raw_payload_blob_links
         WHERE owner_kind = $1 AND owner_id = $2
         RETURNING raw_path",
    )
    .bind(owner_kind)
    .bind(owner_id)
    .fetch_all(&mut *tx)
    .await?;

    for raw_path in released_paths {
        if retained_paths.contains(raw_path.as_str()) {
            // Keep a shared path row alive across owner-link replacement so its file identity
            // remains authoritative instead of being lost to delete-and-reinsert.
            continue;
        }
        sqlx::query(
            "DELETE FROM proxy_raw_payload_blobs AS blob
             WHERE blob.raw_path = $1
               AND NOT EXISTS (
                   SELECT 1
                   FROM proxy_raw_payload_blob_links AS link
                   WHERE link.raw_path = blob.raw_path
               )
               AND NOT EXISTS (
                   SELECT 1
                   FROM archive_raw_reference_changes AS archive_link
                   WHERE archive_link.raw_path = blob.raw_path
                     AND archive_link.action = 'link'
                     AND archive_link.state = 'active'
               )",
        )
        .bind(raw_path)
        .execute(&mut *tx)
        .await?;
    }

    for link in links {
        sqlx::query(
            "INSERT INTO proxy_raw_payload_blobs
                (raw_path, storage_codec, logical_size_bytes)
             VALUES ($1, COALESCE(NULLIF(BTRIM($2), ''), 'identity'), $3)
             ON CONFLICT (raw_path) DO UPDATE SET
                 storage_codec = EXCLUDED.storage_codec,
                 logical_size_bytes = EXCLUDED.logical_size_bytes,
                 updated_at = CURRENT_TIMESTAMP",
        )
        .bind(link.raw_path)
        .bind(link.storage_codec)
        .bind(link.logical_size_bytes)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO proxy_raw_payload_blob_links
                (raw_path, owner_kind, owner_id, raw_role)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (raw_path, owner_kind, owner_id, raw_role) DO NOTHING",
        )
        .bind(link.raw_path)
        .bind(owner_kind)
        .bind(owner_id)
        .bind(link.raw_role)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn replace_postgres_invocation_raw_links(
    tx: &mut PgConnection,
    invocation_id: i64,
    request: PostgresRawPayloadMeta<'_>,
    response: PostgresRawPayloadMeta<'_>,
) -> Result<()> {
    let mut links = Vec::with_capacity(2);
    if let Some(raw_path) = request.raw_path {
        links.push(PostgresRawPayloadLink {
            raw_path,
            storage_codec: request.storage_codec.unwrap_or("identity"),
            logical_size_bytes: request.logical_size_bytes.unwrap_or_default(),
            raw_role: "request",
        });
    }
    if let Some(raw_path) = response.raw_path {
        links.push(PostgresRawPayloadLink {
            raw_path,
            storage_codec: response.storage_codec.unwrap_or("identity"),
            logical_size_bytes: response.logical_size_bytes.unwrap_or_default(),
            raw_role: "response",
        });
    }
    replace_postgres_raw_payload_links(tx, "invocation", invocation_id, &links).await
}

pub(super) async fn replace_postgres_attempt_raw_links(
    tx: &mut PgConnection,
    attempt_id: i64,
    response_raw_path: Option<&str>,
    response_raw_codec: Option<&str>,
    response_raw_size: Option<i64>,
) -> Result<()> {
    let links = response_raw_path
        .map(|raw_path| {
            vec![PostgresRawPayloadLink {
                raw_path,
                storage_codec: response_raw_codec.unwrap_or("identity"),
                logical_size_bytes: response_raw_size.unwrap_or_default(),
                raw_role: "response",
            }]
        })
        .unwrap_or_default();
    replace_postgres_raw_payload_links(tx, "attempt", attempt_id, &links).await
}

pub(super) async fn load_persisted_invocation_identity_postgres_tx(
    tx: &mut PgConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Option<crate::proxy::PersistedInvocationIdentityRow>> {
    sqlx::query_as::<_, crate::proxy::PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status, failure_kind, source, model, input_tokens, output_tokens,
               cache_input_tokens, reported_cache_write_tokens, reasoning_tokens,
               total_tokens, error_message, payload
        FROM codex_invocations
        WHERE invoke_id = $1 AND occurred_at = $2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(Into::into)
}

pub(super) async fn persist_proxy_capture_record_postgres_tx(
    tx: &mut PgConnection,
    capture_started: Instant,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    persist_proxy_capture_postgres_tx(tx, Some(capture_started), record).await
}

pub(super) async fn persist_proxy_capture_runtime_record_postgres_tx(
    tx: &mut PgConnection,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    persist_proxy_capture_postgres_tx(tx, None, record).await
}

async fn persist_proxy_capture_postgres_tx(
    tx: &mut PgConnection,
    capture_started: Option<Instant>,
    mut record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let raw_response = if record.response_body_preview_enabled {
        record.raw_response.clone()
    } else {
        String::new()
    };
    let resp_raw = if record.response_body_preview_enabled {
        record.resp_raw.clone()
    } else {
        RawPayloadMeta {
            path: None,
            size_bytes: record.resp_raw.size_bytes,
            truncated: record.resp_raw.truncated,
            truncated_reason: record.resp_raw.truncated_reason.clone(),
            write_fence: None,
        }
    };
    let failure = resolve_failure_classification(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        None,
        None,
    );
    let failure_kind = failure.failure_kind.clone();
    if let Some(capture_started) = capture_started
        && (!record.timings.t_total_ms.is_finite() || record.timings.t_total_ms <= 0.0)
    {
        record.timings.t_total_ms = elapsed_ms(capture_started);
    }
    let is_terminal_capture = capture_started.is_some();
    let optional_timing = |value: f64| value.is_finite().then_some(value);
    let nonnegative_timing = |value: f64| (value.is_finite() && value >= 0.0).then_some(value);
    let t_total_ms = is_terminal_capture
        .then(|| nonnegative_timing(record.timings.t_total_ms))
        .flatten();
    let t_req_read_ms = optional_timing(record.timings.t_req_read_ms);
    let t_req_parse_ms = optional_timing(record.timings.t_req_parse_ms);
    let t_upstream_connect_ms = optional_timing(record.timings.t_upstream_connect_ms);
    let t_upstream_ttfb_ms = optional_timing(record.timings.t_upstream_ttfb_ms);
    let first_token_ms = record.timings.first_token_ms.and_then(nonnegative_timing);
    let t_upstream_stream_ms = is_terminal_capture
        .then(|| nonnegative_timing(record.timings.t_upstream_stream_ms))
        .flatten();
    let t_resp_parse_ms = is_terminal_capture
        .then(|| nonnegative_timing(record.timings.t_resp_parse_ms))
        .flatten();
    let t_persist_ms = is_terminal_capture
        .then(|| nullable_runtime_timing_value(record.timings.t_persist_ms))
        .flatten();
    let created_at = format_utc_iso_millis(Utc::now());

    let persist_started = Instant::now();
    let persisted = sqlx::query_as::<
        _,
        (
            i64,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<i64>,
        ),
    >(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_codec,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at,
            reported_cache_write_tokens
        )
        VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
            $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28,
            $29, $30, $31, $32, $33, $34, $35, $36, $37, $38, $39, $40, $41,
            $42, $43, $44, $45
        )
        ON CONFLICT (invoke_id, occurred_at) DO UPDATE SET
            source = EXCLUDED.source,
            model = EXCLUDED.model,
            input_tokens = EXCLUDED.input_tokens,
            output_tokens = EXCLUDED.output_tokens,
            cache_input_tokens = EXCLUDED.cache_input_tokens,
            reported_cache_write_tokens = COALESCE(
                EXCLUDED.reported_cache_write_tokens,
                codex_invocations.reported_cache_write_tokens
            ),
            reasoning_tokens = EXCLUDED.reasoning_tokens,
            total_tokens = EXCLUDED.total_tokens,
            cost = EXCLUDED.cost,
            cost_input = EXCLUDED.cost_input,
            cost_cache_write = EXCLUDED.cost_cache_write,
            cost_cache_read = EXCLUDED.cost_cache_read,
            cost_output = EXCLUDED.cost_output,
            cost_reasoning = EXCLUDED.cost_reasoning,
            cost_estimated = EXCLUDED.cost_estimated,
            price_version = EXCLUDED.price_version,
            status = EXCLUDED.status,
            error_message = EXCLUDED.error_message,
            failure_kind = EXCLUDED.failure_kind,
            failure_class = EXCLUDED.failure_class,
            is_actionable = EXCLUDED.is_actionable,
            payload = EXCLUDED.payload,
            raw_response = EXCLUDED.raw_response,
            request_raw_path = EXCLUDED.request_raw_path,
            request_raw_codec = EXCLUDED.request_raw_codec,
            request_raw_size = EXCLUDED.request_raw_size,
            request_raw_truncated = EXCLUDED.request_raw_truncated,
            request_raw_truncated_reason = EXCLUDED.request_raw_truncated_reason,
            response_raw_path = EXCLUDED.response_raw_path,
            response_raw_codec = EXCLUDED.response_raw_codec,
            response_raw_size = EXCLUDED.response_raw_size,
            response_raw_truncated = EXCLUDED.response_raw_truncated,
            response_raw_truncated_reason = EXCLUDED.response_raw_truncated_reason,
            t_total_ms = EXCLUDED.t_total_ms,
            t_req_read_ms = EXCLUDED.t_req_read_ms,
            t_req_parse_ms = EXCLUDED.t_req_parse_ms,
            t_upstream_connect_ms = EXCLUDED.t_upstream_connect_ms,
            t_upstream_ttfb_ms = EXCLUDED.t_upstream_ttfb_ms,
            first_token_ms = EXCLUDED.first_token_ms,
            t_upstream_stream_ms = EXCLUDED.t_upstream_stream_ms,
            t_resp_parse_ms = EXCLUDED.t_resp_parse_ms,
            t_persist_ms = EXCLUDED.t_persist_ms
        WHERE LOWER(TRIM(COALESCE(codex_invocations.status, ''))) IN ('running', 'pending')
           OR (
                LOWER(TRIM(COALESCE(codex_invocations.status, ''))) = 'interrupted'
                AND LOWER(TRIM(COALESCE(codex_invocations.failure_kind, ''))) = 'proxy_interrupted'
           )
        RETURNING id,
                  request_raw_path,
                  request_raw_codec,
                  request_raw_size,
                  response_raw_path,
                  response_raw_codec,
                  response_raw_size
        "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .bind(SOURCE_PROXY)
    .bind(&record.model)
    .bind(record.usage.input_tokens)
    .bind(record.usage.output_tokens)
    .bind(record.usage.cache_input_tokens)
    .bind(record.usage.reasoning_tokens)
    .bind(record.usage.total_tokens)
    .bind(record.cost)
    .bind(record.cost_breakdown.map(|value| value.input))
    .bind(record.cost_breakdown.map(|value| value.cache_write))
    .bind(record.cost_breakdown.map(|value| value.cache_read))
    .bind(record.cost_breakdown.map(|value| value.output))
    .bind(record.cost_breakdown.map(|value| value.reasoning))
    .bind(record.cost_estimated)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure_kind.as_deref())
    .bind(failure.failure_class.as_str())
    .bind(failure.is_actionable)
    .bind(record.payload.as_deref())
    .bind(&raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(raw_payload_meta_codec(&record.req_raw))
    .bind(record.req_raw.size_bytes)
    .bind(record.req_raw.truncated)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(resp_raw.path.as_deref())
    .bind(raw_payload_meta_codec(&resp_raw))
    .bind(resp_raw.size_bytes)
    .bind(resp_raw.truncated)
    .bind(resp_raw.truncated_reason.as_deref())
    .bind(t_total_ms)
    .bind(t_req_read_ms)
    .bind(t_req_parse_ms)
    .bind(t_upstream_connect_ms)
    .bind(t_upstream_ttfb_ms)
    .bind(first_token_ms)
    .bind(t_upstream_stream_ms)
    .bind(t_resp_parse_ms)
    .bind(t_persist_ms)
    .bind(created_at)
    .bind(record.usage.reported_cache_write_tokens)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((
        invocation_id,
        request_raw_path,
        request_raw_codec,
        request_raw_size,
        response_raw_path,
        response_raw_codec,
        response_raw_size,
    )) = persisted
    else {
        let current = sqlx::query_as::<
            _,
            (
                i64,
                Option<String>,
                Option<String>,
                Option<i64>,
                Option<String>,
                Option<String>,
                Option<i64>,
            ),
        >(
            "SELECT id, request_raw_path, request_raw_codec, request_raw_size,
                    response_raw_path, response_raw_codec, response_raw_size
             FROM codex_invocations
             WHERE invoke_id = $1 AND occurred_at = $2
             ORDER BY id DESC
             LIMIT 1",
        )
        .bind(&record.invoke_id)
        .bind(&record.occurred_at)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((
            invocation_id,
            request_raw_path,
            request_raw_codec,
            request_raw_size,
            response_raw_path,
            response_raw_codec,
            response_raw_size,
        )) = current
        {
            replace_postgres_invocation_raw_links(
                tx,
                invocation_id,
                PostgresRawPayloadMeta {
                    raw_path: request_raw_path.as_deref(),
                    storage_codec: request_raw_codec.as_deref(),
                    logical_size_bytes: request_raw_size,
                },
                PostgresRawPayloadMeta {
                    raw_path: response_raw_path.as_deref(),
                    storage_codec: response_raw_codec.as_deref(),
                    logical_size_bytes: response_raw_size,
                },
            )
            .await?;
        }
        return Ok(None);
    };
    replace_postgres_invocation_raw_links(
        tx,
        invocation_id,
        PostgresRawPayloadMeta {
            raw_path: request_raw_path.as_deref(),
            storage_codec: request_raw_codec.as_deref(),
            logical_size_bytes: request_raw_size,
        },
        PostgresRawPayloadMeta {
            raw_path: response_raw_path.as_deref(),
            storage_codec: response_raw_codec.as_deref(),
            logical_size_bytes: response_raw_size,
        },
    )
    .await?;
    let measured_t_persist_ms = elapsed_ms(persist_started);
    sqlx::query("UPDATE codex_invocations SET t_persist_ms = $2 WHERE id = $1")
        .bind(invocation_id)
        .bind(measured_t_persist_ms)
        .execute(&mut *tx)
        .await?;

    let mut persisted = api_invocation_from_runtime_record(&record);
    persisted.id = invocation_id;
    Ok(Some(persisted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::config::StorageConfig;
    use crate::storage::core::{
        BackendKind, ModuleAuthority, ModuleGenerationGate, StorageModule, StoragePools,
    };
    use crate::storage::traffic::TrafficFactStore;
    use sqlx::{PgConnection, SqlitePool};

    async fn test_authority(database_url: String) -> Result<TrafficFactStore> {
        let config = StorageConfig {
            postgres_database_url: Some(database_url),
            migration_admin_token: None,
            postgres_max_connections: 1,
            migration_switch_budget: std::time::Duration::from_secs(3),
        };
        let storage = StoragePools::from_sqlite_and_config(
            SqlitePool::connect("sqlite::memory:").await?,
            &config,
        )
        .await?;
        Ok(TrafficFactStore::with_authority(
            storage,
            ModuleGenerationGate::new(
                StorageModule::TrafficArchive,
                ModuleAuthority {
                    backend: BackendKind::Postgres,
                    generation: 1,
                },
            ),
        ))
    }

    async fn create_raw_link_fixture(connection: &mut PgConnection) -> Result<()> {
        for statement in [
            r#"
            CREATE TEMP TABLE codex_invocations (
                id BIGSERIAL PRIMARY KEY,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL,
                model TEXT,
                input_tokens BIGINT,
                output_tokens BIGINT,
                cache_input_tokens BIGINT,
                reasoning_tokens BIGINT,
                total_tokens BIGINT,
                cost DOUBLE PRECISION,
                cost_input DOUBLE PRECISION,
                cost_cache_write DOUBLE PRECISION,
                cost_cache_read DOUBLE PRECISION,
                cost_output DOUBLE PRECISION,
                cost_reasoning DOUBLE PRECISION,
                cost_estimated BOOLEAN NOT NULL DEFAULT FALSE,
                price_version TEXT,
                status TEXT,
                error_message TEXT,
                failure_kind TEXT,
                failure_class TEXT,
                is_actionable BOOLEAN NOT NULL DEFAULT FALSE,
                payload TEXT,
                raw_response TEXT NOT NULL DEFAULT '',
                request_raw_path TEXT,
                request_raw_codec TEXT,
                request_raw_size BIGINT,
                request_raw_truncated BOOLEAN NOT NULL DEFAULT FALSE,
                request_raw_truncated_reason TEXT,
                response_raw_path TEXT,
                response_raw_codec TEXT,
                response_raw_size BIGINT,
                response_raw_truncated BOOLEAN NOT NULL DEFAULT FALSE,
                response_raw_truncated_reason TEXT,
                t_total_ms DOUBLE PRECISION,
                t_req_read_ms DOUBLE PRECISION,
                t_req_parse_ms DOUBLE PRECISION,
                t_upstream_connect_ms DOUBLE PRECISION,
                t_upstream_ttfb_ms DOUBLE PRECISION,
                first_token_ms DOUBLE PRECISION,
                t_upstream_stream_ms DOUBLE PRECISION,
                t_resp_parse_ms DOUBLE PRECISION,
                t_persist_ms DOUBLE PRECISION,
                created_at TEXT,
                reported_cache_write_tokens BIGINT,
                UNIQUE (invoke_id, occurred_at)
            )
            "#,
            r#"
            CREATE TEMP TABLE pool_upstream_request_attempts (
                id BIGSERIAL PRIMARY KEY,
                attempt_public_id TEXT NOT NULL,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                endpoint TEXT NOT NULL,
                route_mode TEXT NOT NULL,
                sticky_key TEXT,
                routing_source TEXT,
                routing_selection_audit_json TEXT,
                upstream_base_url_host TEXT,
                group_name_snapshot TEXT,
                proxy_binding_key_snapshot TEXT,
                request_model TEXT,
                upstream_request_model TEXT,
                model_mapping_pattern TEXT,
                upstream_account_id BIGINT NOT NULL,
                upstream_route_key TEXT NOT NULL,
                attempt_index BIGINT NOT NULL,
                distinct_account_index BIGINT NOT NULL,
                same_account_retry_index BIGINT NOT NULL,
                requester_ip TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                status TEXT NOT NULL,
                phase TEXT,
                http_status BIGINT,
                downstream_http_status BIGINT,
                failure_kind TEXT,
                error_message TEXT,
                downstream_error_message TEXT,
                connect_latency_ms DOUBLE PRECISION,
                first_byte_latency_ms DOUBLE PRECISION,
                stream_latency_ms DOUBLE PRECISION,
                upstream_request_id TEXT,
                upstream_request_compression_algorithm TEXT,
                upstream_request_compression_mode TEXT,
                upstream_request_logical_body_bytes BIGINT,
                upstream_request_transmitted_body_bytes BIGINT,
                upstream_request_header_bytes_approx BIGINT,
                upstream_response_body_bytes BIGINT,
                upstream_response_header_bytes_approx BIGINT,
                compact_support_status TEXT,
                compact_support_reason TEXT,
                request_summary_json TEXT,
                response_raw_path TEXT,
                response_raw_codec TEXT,
                response_raw_size BIGINT,
                response_raw_truncated BOOLEAN NOT NULL DEFAULT FALSE,
                response_raw_truncated_reason TEXT,
                response_content_encoding TEXT,
                invocation_id BIGINT
            )
            "#,
            r#"
            CREATE UNIQUE INDEX pool_upstream_request_attempts_public_id_unique
            ON pool_upstream_request_attempts (attempt_public_id)
            WHERE attempt_public_id IS NOT NULL
            "#,
            r#"
            CREATE TEMP TABLE proxy_raw_payload_blobs (
                raw_path TEXT PRIMARY KEY,
                storage_codec TEXT NOT NULL DEFAULT 'identity',
                logical_size_bytes BIGINT NOT NULL DEFAULT 0,
                file_identity_sha256 TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
            r#"
            CREATE TEMP TABLE proxy_raw_payload_blob_links (
                id BIGSERIAL PRIMARY KEY,
                raw_path TEXT NOT NULL REFERENCES proxy_raw_payload_blobs(raw_path) ON DELETE CASCADE,
                owner_kind TEXT NOT NULL CHECK (owner_kind IN ('invocation', 'attempt')),
                owner_id BIGINT NOT NULL,
                raw_role TEXT NOT NULL CHECK (raw_role IN ('request', 'response')),
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE (raw_path, owner_kind, owner_id, raw_role)
            )
            "#,
            r#"
            CREATE TEMP TABLE archive_raw_reference_changes (
                archive_batch_id BIGINT NOT NULL,
                raw_path TEXT NOT NULL,
                owner_kind TEXT NOT NULL,
                owner_id BIGINT NOT NULL,
                raw_role TEXT NOT NULL,
                action TEXT NOT NULL,
                raw_identity_sha256 TEXT,
                state TEXT NOT NULL
            )
            "#,
        ] {
            sqlx::query(statement).execute(&mut *connection).await?;
        }
        Ok(())
    }

    fn raw_meta(path: Option<&str>) -> RawPayloadMeta {
        RawPayloadMeta {
            path: path.map(ToOwned::to_owned),
            size_bytes: path.map_or(0, |_| 11),
            truncated: false,
            truncated_reason: None,
            write_fence: None,
        }
    }

    fn capture_record(
        invoke_id: &str,
        occurred_at: &str,
        status: &str,
        request_raw_path: Option<&str>,
        response_raw_path: Option<&str>,
    ) -> ProxyCaptureRecord {
        ProxyCaptureRecord {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            model: Some("gpt-5.2-codex".to_string()),
            usage: ParsedUsage::default(),
            cost: None,
            cost_breakdown: None,
            cost_estimated: false,
            price_version: None,
            status: status.to_string(),
            error_message: None,
            failure_kind: None,
            payload: Some("{}".to_string()),
            raw_response: "{}".to_string(),
            response_body_preview_enabled: true,
            req_raw: raw_meta(request_raw_path),
            resp_raw: raw_meta(response_raw_path),
            timings: StageTimings {
                t_total_ms: 1.0,
                t_req_read_ms: 1.0,
                t_req_parse_ms: 1.0,
                t_upstream_connect_ms: 1.0,
                t_upstream_ttfb_ms: 1.0,
                first_token_ms: None,
                t_upstream_stream_ms: 1.0,
                t_resp_parse_ms: 1.0,
                t_persist_ms: 1.0,
            },
        }
    }

    fn attempt_snapshot(response_raw_path: Option<&str>) -> RuntimeAttemptSnapshot {
        RuntimeAttemptSnapshot {
            attempt_public_id: "attempt-1".to_string(),
            routing_control_fingerprint: None,
            invoke_id: "raw-link-invocation".to_string(),
            occurred_at: "2026-10-26 00:00:00".to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: None,
            routing_source: None,
            routing_selection_audit_json: None,
            requester_ip: None,
            upstream_base_url_host: Some("api.example.com".to_string()),
            group_name_snapshot: None,
            proxy_binding_key_snapshot: None,
            request_model: None,
            upstream_request_model: None,
            model_mapping_pattern: None,
            request_summary_json: None,
            upstream_account_id: 17,
            upstream_route_key: "route-1".to_string(),
            attempt_index: 0,
            distinct_account_index: 0,
            same_account_retry_index: 0,
            started_at: "2026-10-26 00:00:00".to_string(),
            finished_at: Some("2026-10-26 00:00:01".to_string()),
            status: "success".to_string(),
            phase: Some("complete".to_string()),
            http_status: Some(200),
            downstream_http_status: Some(200),
            failure_kind: None,
            error_message: None,
            downstream_error_message: None,
            connect_latency_ms: Some(1.0),
            first_byte_latency_ms: Some(2.0),
            stream_latency_ms: Some(3.0),
            upstream_request_id: None,
            compact_support_status: None,
            compact_support_reason: None,
            upstream_request_compression_algorithm: None,
            upstream_request_compression_mode: None,
            upstream_request_logical_body_bytes: None,
            upstream_request_transmitted_body_bytes: None,
            upstream_request_header_bytes_approx: None,
            upstream_response_body_bytes: None,
            upstream_response_header_bytes_approx: None,
            response_raw_path: response_raw_path.map(ToOwned::to_owned),
            response_raw_codec: response_raw_path.map(|_| "identity".to_string()),
            response_raw_size: response_raw_path.map(|_| 13),
            response_raw_truncated: false,
            response_raw_truncated_reason: None,
            response_content_encoding: None,
        }
    }

    async fn raw_links(
        connection: &mut PgConnection,
        owner_kind: &str,
        owner_id: i64,
    ) -> Result<Vec<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT raw_path, raw_role
             FROM proxy_raw_payload_blob_links
             WHERE owner_kind = $1 AND owner_id = $2
             ORDER BY raw_role, raw_path",
        )
        .bind(owner_kind)
        .bind(owner_id)
        .fetch_all(&mut *connection)
        .await?)
    }

    async fn blob_count(connection: &mut PgConnection) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM proxy_raw_payload_blobs")
                .fetch_one(&mut *connection)
                .await?,
        )
    }

    async fn authority_blob_count(authority: &TrafficFactStore) -> Result<i64> {
        let mut transaction = authority.begin().await?;
        let count = blob_count(transaction.postgres_connection()?).await?;
        transaction.rollback().await?;
        Ok(count)
    }

    #[tokio::test]
    async fn optional_postgres_raw_link_lifecycle_is_atomic_and_idempotent() -> Result<()> {
        let Ok(database_url) = std::env::var("POSTGRES_TEST_DATABASE_URL") else {
            return Ok(());
        };
        let authority = test_authority(database_url).await?;
        let mut setup = authority.begin().await?;
        create_raw_link_fixture(setup.postgres_connection()?).await?;
        setup.commit().await?;

        let mut transaction = authority.begin().await?;
        let first = persist_proxy_capture_runtime_record_postgres_tx(
            transaction.postgres_connection()?,
            capture_record(
                "raw-link-invocation",
                "2026-10-26 00:00:00",
                "running",
                Some("request-a.raw"),
                Some("response-a.raw"),
            ),
        )
        .await?
        .expect("first invocation insert must return an identity");
        let invocation_id = first.id;
        assert_eq!(
            raw_links(
                transaction.postgres_connection()?,
                "invocation",
                invocation_id,
            )
            .await?,
            vec![
                ("request-a.raw".to_string(), "request".to_string()),
                ("response-a.raw".to_string(), "response".to_string()),
            ]
        );
        transaction.commit().await?;

        let mut archive_reference = authority.begin().await?;
        sqlx::query(
            "INSERT INTO archive_raw_reference_changes
                (archive_batch_id, raw_path, owner_kind, owner_id, raw_role, action,
                 raw_identity_sha256, state)
             VALUES (9001, 'request-a.raw', 'invocation', $1, 'request', 'link',
                     'archive-request-a', 'active')",
        )
        .bind(invocation_id)
        .execute(archive_reference.postgres_connection()?)
        .await?;
        archive_reference.commit().await?;

        let mut transaction = authority.begin().await?;
        persist_proxy_capture_runtime_record_postgres_tx(
            transaction.postgres_connection()?,
            capture_record(
                "raw-link-invocation",
                "2026-10-26 00:00:00",
                "running",
                Some("request-b.raw"),
                Some("response-b.raw"),
            ),
        )
        .await?;
        assert_eq!(
            raw_links(
                transaction.postgres_connection()?,
                "invocation",
                invocation_id,
            )
            .await?,
            vec![
                ("request-b.raw".to_string(), "request".to_string()),
                ("response-b.raw".to_string(), "response".to_string()),
            ]
        );
        transaction.commit().await?;
        assert_eq!(authority_blob_count(&authority).await?, 3);

        let mut transaction = authority.begin().await?;
        persist_proxy_capture_runtime_record_postgres_tx(
            transaction.postgres_connection()?,
            capture_record(
                "raw-link-invocation",
                "2026-10-26 00:00:00",
                "running",
                None,
                None,
            ),
        )
        .await?;
        assert!(
            raw_links(
                transaction.postgres_connection()?,
                "invocation",
                invocation_id,
            )
            .await?
            .is_empty()
        );
        transaction.commit().await?;
        assert_eq!(authority_blob_count(&authority).await?, 1);
        let mut verification = authority.begin().await?;
        let retained_archive_blob: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM proxy_raw_payload_blobs WHERE raw_path = 'request-a.raw'",
        )
        .fetch_one(verification.postgres_connection()?)
        .await?;
        assert_eq!(retained_archive_blob, 1);
        verification.rollback().await?;

        let mut transaction = authority.begin().await?;
        persist_proxy_capture_runtime_record_postgres_tx(
            transaction.postgres_connection()?,
            capture_record(
                "raw-link-invocation",
                "2026-10-26 00:00:00",
                "success",
                Some("request-c.raw"),
                Some("response-c.raw"),
            ),
        )
        .await?;
        transaction.commit().await?;

        let mut identity_transaction = authority.begin().await?;
        sqlx::query(
            "UPDATE proxy_raw_payload_blobs
             SET file_identity_sha256 = CASE raw_path
                 WHEN 'request-c.raw' THEN 'identity-request-c'
                 WHEN 'response-c.raw' THEN 'identity-response-c'
             END
             WHERE raw_path IN ('request-c.raw', 'response-c.raw')",
        )
        .execute(identity_transaction.postgres_connection()?)
        .await?;
        identity_transaction.commit().await?;

        let mut transaction = authority.begin().await?;
        let duplicate = persist_proxy_capture_runtime_record_postgres_tx(
            transaction.postgres_connection()?,
            capture_record(
                "raw-link-invocation",
                "2026-10-26 00:00:00",
                "success",
                Some("request-d.raw"),
                Some("response-d.raw"),
            ),
        )
        .await?;
        assert!(duplicate.is_none());
        assert_eq!(
            raw_links(
                transaction.postgres_connection()?,
                "invocation",
                invocation_id,
            )
            .await?,
            vec![
                ("request-c.raw".to_string(), "request".to_string()),
                ("response-c.raw".to_string(), "response".to_string()),
            ]
        );
        let identities = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT raw_path, file_identity_sha256
             FROM proxy_raw_payload_blobs
             WHERE raw_path IN ('request-c.raw', 'response-c.raw')
             ORDER BY raw_path",
        )
        .fetch_all(transaction.postgres_connection()?)
        .await?;
        assert_eq!(
            identities,
            vec![
                (
                    "request-c.raw".to_string(),
                    Some("identity-request-c".to_string())
                ),
                (
                    "response-c.raw".to_string(),
                    Some("identity-response-c".to_string())
                ),
            ]
        );
        transaction.commit().await?;

        let mut transaction = authority.begin().await?;
        let attempt = attempt_snapshot(Some("attempt-a.raw"));
        super::super::terminal_attempts::persist_runtime_attempt_snapshot_postgres_tx_with_invocation_id(
            transaction.postgres_connection()?,
            &attempt,
            Some(invocation_id),
        )
        .await?;
        let attempt_id: i64 = sqlx::query_scalar(
            "SELECT id FROM pool_upstream_request_attempts WHERE attempt_public_id = $1",
        )
        .bind(&attempt.attempt_public_id)
        .fetch_one(transaction.postgres_connection()?)
        .await?;
        assert_eq!(
            raw_links(transaction.postgres_connection()?, "attempt", attempt_id).await?,
            vec![("attempt-a.raw".to_string(), "response".to_string())]
        );
        transaction.commit().await?;

        let mut transaction = authority.begin().await?;
        super::super::terminal_attempts::persist_runtime_attempt_snapshot_postgres_tx_with_invocation_id(
            transaction.postgres_connection()?,
            &attempt_snapshot(Some("attempt-b.raw")),
            Some(invocation_id),
        )
        .await?;
        assert_eq!(
            raw_links(transaction.postgres_connection()?, "attempt", attempt_id).await?,
            vec![("attempt-b.raw".to_string(), "response".to_string())]
        );
        transaction.commit().await?;

        let mut transaction = authority.begin().await?;
        super::super::attempt_progress::persist_postgres(
            transaction.postgres_transaction()?,
            &BatchedAttemptProgress {
                attempt_id,
                pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
                response_raw_path: Some("attempt-p2.raw".to_string()),
                response_raw_codec: Some("identity".to_string()),
                response_raw_size: Some(17),
                response_raw_truncated: Some(false),
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(
            raw_links(transaction.postgres_connection()?, "attempt", attempt_id).await?,
            vec![("attempt-p2.raw".to_string(), "response".to_string())]
        );
        transaction.commit().await?;

        let mut transaction = authority.begin().await?;
        persist_proxy_capture_runtime_record_postgres_tx(
            transaction.postgres_connection()?,
            capture_record(
                "rolled-back-invocation",
                "2026-10-26 00:00:00",
                "running",
                Some("rollback-request.raw"),
                Some("rollback-response.raw"),
            ),
        )
        .await?;
        assert_eq!(blob_count(transaction.postgres_connection()?).await?, 5);
        transaction.rollback().await?;
        let mut verification = authority.begin().await?;
        let rolled_back_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = $1")
                .bind("rolled-back-invocation")
                .fetch_one(verification.postgres_connection()?)
                .await?;
        assert_eq!(rolled_back_count, 0);
        assert_eq!(blob_count(verification.postgres_connection()?).await?, 3);
        verification.rollback().await?;
        Ok(())
    }
}
