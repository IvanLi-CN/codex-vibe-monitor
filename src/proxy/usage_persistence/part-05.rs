pub(crate) async fn persist_and_broadcast_proxy_capture_terminal_record(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let enqueue_started = Instant::now();
    let persisted_record = api_invocation_from_runtime_record(&record);
    let invoke_id = persisted_record.invoke_id.clone();
    let duplicate_terminal = remove_proxy_runtime_snapshot_for_terminal(state, &persisted_record);
    if duplicate_terminal {
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %persisted_record.occurred_at,
            business_unblocked_record_write = true,
            "duplicate terminal proxy capture record skipped before sqlite enqueue"
        );
        schedule_proxy_capture_follow_up_after_terminal_enqueue(
            state,
            &invoke_id,
            "duplicate_runtime_terminal",
        );
        return Ok(());
    }
    observe_successful_proxy_capture_model_route_cache(state, &record).await;
    let projection = register_terminal_projection_before_enqueue(state, &persisted_record).await;
    let delta = &projection.dashboard;
    let startup_backfill_tasks = startup_backfill_tasks_for_terminal(&persisted_record);
    debug!(
        invoke_id = %invoke_id,
        terminal_delta_applied_selection_count = delta.applied_selection_count,
        terminal_delta_duplicate = delta.duplicate,
        terminal_delta_skipped_out_of_range_count = delta.skipped_out_of_range_count,
        response_source = "memory",
        "registered terminal record in dashboard activity read model before sqlite enqueue"
    );
    let terminal_enqueue =
        state
            .sqlite_batch_writer
            .enqueue_terminal(BatchedTerminalInvocationWrite {
                record,
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: delta.terminal_sequence,
                terminal_projection_event_ids: projection.event_id.into_iter().collect(),
                startup_backfill_tasks,
            });
    let terminal_enqueued = terminal_enqueue.enqueued;
    if !terminal_enqueued {
        rollback_terminal_projection_before_enqueue(state, &persisted_record, &projection).await;
        let terminal_tombstone_cleared = state
            .proxy_runtime_invocations
            .clear_terminal_tombstone(&persisted_record.invoke_id, &persisted_record.occurred_at);
        warn!(
            invoke_id = %invoke_id,
            occurred_at = %persisted_record.occurred_at,
            enqueue_failed_by_class = "terminal_invocation",
            terminal_tombstone_cleared,
            durability_mode = terminal_enqueue.durability_mode.as_str(),
            journal_sequence = ?terminal_enqueue.journal_sequence,
            business_unblocked_record_write = true,
            record_flush_deferred_or_failed = "terminal_invocation_enqueue_failed",
            "terminal proxy capture record dropped by sqlite write controller"
        );
    } else {
        debug!(
            invoke_id = %invoke_id,
            terminal_record_enqueue_elapsed = enqueue_started.elapsed().as_millis() as u64,
            durability_mode = terminal_enqueue.durability_mode.as_str(),
            journal_sequence = ?terminal_enqueue.journal_sequence,
            journal_pending_records = terminal_enqueue.journal_pending_records,
            journal_pending_bytes = terminal_enqueue.journal_pending_bytes,
            business_unblocked_record_write = true,
            record_flush_deferred_or_failed = "terminal_invocation_enqueued_async",
            "terminal proxy capture record queued for sqlite write controller"
        );
    }
    #[cfg(test)]
    if terminal_enqueued && state.sqlite_batch_writer.auto_flush_terminal_for_test() {
        state
            .sqlite_batch_writer
            .flush_buffered_for_test(&state.pool)
            .await;
    }
    if terminal_enqueued {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                &persisted_record,
                RuntimeMutationKind::TerminalCommitted,
            ));
        #[cfg(test)]
        broadcast_test_record_payload(state, &persisted_record);
    }
    if terminal_enqueued {
        schedule_dashboard_activity_live_snapshot(state);
        schedule_proxy_capture_follow_up_after_terminal_enqueue(
            state,
            &invoke_id,
            "runtime_terminal",
        );
    }

    Ok(())
}

pub(crate) async fn persist_proxy_capture_runtime_record(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    persist_proxy_capture_runtime_record_core(pool, record, true).await
}

pub(crate) async fn persist_proxy_capture_runtime_record_core(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
) -> Result<Option<ApiInvocation>> {
    let mut tx = pool.begin().await?;
    let persisted =
        persist_proxy_capture_runtime_record_tx(tx.as_mut(), record, write_derived_inline).await?;
    tx.commit().await?;
    Ok(persisted)
}

pub(crate) async fn persist_proxy_capture_runtime_record_tx(
    tx: &mut SqliteConnection,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
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
    let t_req_read_ms = nullable_runtime_timing_value(record.timings.t_req_read_ms);
    let t_req_parse_ms = nullable_runtime_timing_value(record.timings.t_req_parse_ms);
    let t_upstream_connect_ms = nullable_runtime_timing_value(record.timings.t_upstream_connect_ms);
    let t_upstream_ttfb_ms = nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms);
    let first_token_ms = record
        .timings
        .first_token_ms
        .filter(|value| value.is_finite() && *value >= 0.0);
    let core_write_started = Instant::now();
    let created_at = format_utc_iso_millis(Utc::now());
    let mut core_write_path = "insert_missing";
    let existing_identity =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?;
    if let Some(existing) = existing_identity.as_ref()
        && !persisted_invocation_allows_proxy_record_update(
            existing.status.as_deref(),
            existing.failure_kind.as_deref(),
            &record.status,
        )
    {
        return Ok(None);
    }

    if let Some(existing) = existing_identity.as_ref() {
        let updated = update_existing_proxy_invocation_record_tx(
            &mut *tx,
            existing.id,
            &record,
            &raw_response,
            &resp_raw,
            failure_kind.as_deref(),
            failure.failure_class.as_str(),
            failure.is_actionable,
            None,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms,
            None,
            None,
            None,
        )
        .await?;
        if !updated {
            return Ok(None);
        }
        core_write_path = "update_existing";
    } else {
        let insert_result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO codex_invocations (
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
                created_at
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36,
                ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44
            )
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
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(&raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&record.req_raw))
        .bind(record.req_raw.size_bytes)
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(resp_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&resp_raw))
        .bind(resp_raw.size_bytes)
        .bind(resp_raw.truncated as i64)
        .bind(resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(first_token_ms)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
        if insert_result.rows_affected() == 0 {
            let Some(existing) = load_persisted_invocation_identity_tx(
                &mut *tx,
                &record.invoke_id,
                &record.occurred_at,
            )
            .await?
            else {
                return Ok(None);
            };
            if !persisted_invocation_allows_proxy_record_update(
                existing.status.as_deref(),
                existing.failure_kind.as_deref(),
                &record.status,
            ) {
                return Ok(None);
            }
            let updated = update_existing_proxy_invocation_record_tx(
                &mut *tx,
                existing.id,
                &record,
                &raw_response,
                &resp_raw,
                failure_kind.as_deref(),
                failure.failure_class.as_str(),
                failure.is_actionable,
                None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                None,
                None,
                None,
            )
            .await?;
            if !updated {
                return Ok(None);
            }
            core_write_path = "update_race";
        }
    }

    let persisted_identity =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?
            .ok_or_else(|| {
                anyhow!("persisted proxy runtime invocation row disappeared after upsert")
            })?;
    if write_derived_inline {
        upsert_invocation_hourly_rollups_tx(
            &mut *tx,
            &[InvocationHourlySourceRecord {
                id: persisted_identity.id,
                occurred_at: record.occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some(record.status.clone()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: record.model.clone(),
                input_tokens: record.usage.input_tokens,
                output_tokens: record.usage.output_tokens,
                cache_input_tokens: record.usage.cache_input_tokens,
                reasoning_tokens: record.usage.reasoning_tokens,
                total_tokens: record.usage.total_tokens,
                cost: record.cost,
                upstream_account_id: crate::proxy::upstream_account_id_from_payload(
                    record.payload.as_deref(),
                ),
                cost_input: record.cost_breakdown.map(|value| value.input),
                cost_cache_write: record.cost_breakdown.map(|value| value.cache_write),
                cost_cache_read: record.cost_breakdown.map(|value| value.cache_read),
                cost_output: record.cost_breakdown.map(|value| value.output),
                cost_reasoning: record.cost_breakdown.map(|value| value.reasoning),
                error_message: record.error_message.clone(),
                failure_kind: failure_kind.clone(),
                failure_class: Some(failure.failure_class.as_str().to_string()),
                is_actionable: Some(failure.is_actionable as i64),
                payload: record.payload.clone(),
                t_total_ms: None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            }],
            &INVOCATION_HOURLY_ROLLUP_TARGETS,
        )
        .await?;
        save_hourly_rollup_live_progress_tx(
            &mut *tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            persisted_identity.id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            &mut *tx,
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
    }

    let persisted =
        load_persisted_api_invocation_tx(&mut *tx, &record.invoke_id, &record.occurred_at).await?;

    let core_write_elapsed_ms = core_write_started.elapsed().as_millis() as u64;
    if core_write_elapsed_ms >= 1_000 {
        warn!(
            invoke_id = %record.invoke_id,
            status = %record.status,
            core_write_path,
            request_raw_bytes = record.req_raw.size_bytes,
            response_raw_bytes = resp_raw.size_bytes,
            has_request_raw_path = record.req_raw.path.is_some(),
            has_response_raw_path = resp_raw.path.is_some(),
            elapsed_ms = core_write_elapsed_ms,
            "proxy capture core invocation write was slow"
        );
    } else {
        debug!(
            invoke_id = %record.invoke_id,
            status = %record.status,
            core_write_path,
            request_raw_bytes = record.req_raw.size_bytes,
            response_raw_bytes = resp_raw.size_bytes,
            has_request_raw_path = record.req_raw.path.is_some(),
            has_response_raw_path = resp_raw.path.is_some(),
            elapsed_ms = core_write_elapsed_ms,
            "proxy capture core invocation write completed"
        );
    }

    Ok(Some(persisted))
}

pub(crate) fn build_running_proxy_capture_record(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    pool_route_active: bool,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
    upstream_account_kind: Option<&str>,
    upstream_base_url_host: Option<&str>,
    proxy_display_name: Option<&str>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&str>,
    response_content_encoding: Option<&str>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    t_upstream_connect_ms: f64,
    t_upstream_ttfb_ms: f64,
) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: request_info.model.clone(),
        usage: ParsedUsage::default(),
        cost: None,
        cost_breakdown: None,
        cost_estimated: false,
        price_version: None,
        status: "running".to_string(),
        error_message: None,
        failure_kind: None,
        payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
            target,
            status: StatusCode::OK,
            is_stream: request_info.is_stream,
            request_contains_encrypted_content: request_info.contains_encrypted_content,
            response_contains_encrypted_content: false,
            compaction_request_kind: request_info.compaction_request_kind,
            compaction_response_kind: None,
            image_intent: request_info.image_intent.as_deref(),
            request_model: request_info.model.as_deref(),
            requested_service_tier: request_info.requested_service_tier.as_deref(),
            billing_service_tier: None,
            reasoning_effort: request_info.reasoning_effort.as_deref(),
            response_model: None,
            usage_missing_reason: None,
            request_parse_error: request_info.parse_error.as_deref(),
            request_compression_algorithm: None,
            request_compression_mode: None,
            request_compression_logical_body_bytes: None,
            request_compression_transmitted_body_bytes: None,
            request_compression_transmission_complete: None,
            failure_kind: None,
            requester_ip,
            request_user_agent: None,
            request_x_forwarded_for: None,
            request_forwarded: None,
            request_x_real_ip: None,
            upstream_scope: if pool_route_active {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_route_active {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key,
            prompt_cache_key,
            prompt_cache_key_attribution_source: request_info
                .prompt_cache_key_attribution_source
                .as_deref(),
            client_fingerprint: None,
            client_header_fingerprints: None,
            upstream_account_id,
            upstream_account_name,
            upstream_account_kind,
            upstream_base_url_host,
            oauth_account_header_attached: None,
            oauth_account_id_shape: None,
            oauth_forwarded_header_count: None,
            oauth_forwarded_header_names: None,
            oauth_fingerprint_version: None,
            oauth_forwarded_header_fingerprints: None,
            oauth_prompt_cache_header_forwarded: None,
            oauth_request_body_prefix_fingerprint: None,
            oauth_request_body_prefix_bytes: None,
            oauth_request_body_snapshot_kind: None,
            oauth_responses_body_mode: None,
            oauth_responses_rewrite: None,
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_status_code: None,
            downstream_error_message: None,
            upstream_request_id: None,
            response_content_encoding,
            stream_failure_origin: None,
            upstream_read_error_kind: None,
            content_encoding_chain: None,
            forwarded_chunk_count: None,
            forwarded_bytes: None,
            usage_observed: None,
            downstream_close_phase: None,
            downstream_write_error_kind: None,
            last_upstream_chunk_gap_ms: None,
            upstream_approx_upload_bytes: None,
            upstream_approx_download_bytes: None,
            proxy_display_name,
            proxy_weight_delta: None,
            pool_attempt_count,
            pool_distinct_account_count,
            pool_attempt_terminal_reason,
            blocked_binding: None,
        })),
        raw_response: "{}".to_string(),
        response_body_preview_enabled: false,
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms: None,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    }
}

pub(crate) fn build_admitted_proxy_capture_runtime_snapshot(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
) -> ProxyCaptureRecord {
    let request_info = RequestCaptureInfo {
        sticky_key: sticky_key.map(ToOwned::to_owned),
        prompt_cache_key: prompt_cache_key.map(ToOwned::to_owned),
        prompt_cache_key_attribution_source: prompt_cache_key.map(|_| "request".to_string()),
        ..RequestCaptureInfo::default()
    };
    build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        target,
        &request_info,
        requester_ip,
        sticky_key,
        prompt_cache_key,
        true,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        0.0,
        0.0,
        0.0,
        0.0,
    )
}

pub(crate) fn resolve_invocation_proxy_display_name(
    selected_proxy: Option<&SelectedForwardProxy>,
) -> Option<String> {
    selected_proxy.map(|proxy| proxy.display_name.clone())
}

pub(crate) fn summarize_response_content_encoding(content_encoding: Option<&str>) -> String {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        "identity".to_string()
    } else {
        encodings.join(", ")
    }
}

#[derive(Default)]
pub(crate) struct RawResponsePreviewBuffer {
    bytes: Vec<u8>,
}

impl RawResponsePreviewBuffer {
    pub(crate) fn append(&mut self, chunk: &[u8]) {
        let remaining = RAW_RESPONSE_PREVIEW_LIMIT.saturating_sub(self.bytes.len());
        if remaining == 0 || chunk.is_empty() {
            return;
        }
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn into_preview(self) -> String {
        build_raw_response_preview(&self.bytes)
    }
}

pub(crate) struct BoundedResponseParseBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded_limit: bool,
}

impl BoundedResponseParseBuffer {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded_limit: false,
        }
    }

    pub(crate) fn append(&mut self, chunk: &[u8]) {
        if self.exceeded_limit || chunk.is_empty() {
            return;
        }

        let remaining = self.limit.saturating_sub(self.bytes.len());
        let take_len = remaining.min(chunk.len());
        if take_len > 0 {
            self.bytes.extend_from_slice(&chunk[..take_len]);
        }
        if take_len < chunk.len() {
            self.exceeded_limit = true;
        }
    }

    pub(crate) fn into_response_info(
        self,
        target: ProxyCaptureTarget,
        content_encoding: Option<&str>,
    ) -> ResponseCaptureInfo {
        let mut response_info =
            parse_target_response_payload(target, &self.bytes, false, content_encoding);
        if self.exceeded_limit {
            merge_response_capture_reason(
                &mut response_info,
                PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
            );
        }
        response_info
    }
}

pub(crate) enum PendingRawPayloadWrite {
    Ready(RawPayloadMeta),
    Task(JoinHandle<RawPayloadMeta>),
}

impl PendingRawPayloadWrite {
    pub(crate) async fn finish(self) -> RawPayloadMeta {
        match self {
            Self::Ready(meta) => meta,
            Self::Task(handle) => match handle.await {
                Ok(meta) => meta,
                Err(err) => RawPayloadMeta {
                    path: None,
                    size_bytes: 0,
                    truncated: true,
                    truncated_reason: Some(format!("write_failed:{err}")),
                },
            },
        }
    }
}

pub(crate) fn spawn_raw_payload_file_write(
    state: &AppState,
    invoke_id: &str,
    kind: &'static str,
    bytes: Bytes,
    enabled: bool,
) -> PendingRawPayloadWrite {
    if bytes.is_empty() {
        return PendingRawPayloadWrite::Ready(RawPayloadMeta::default());
    }
    if !enabled {
        return PendingRawPayloadWrite::Ready(RawPayloadMeta {
            path: None,
            size_bytes: bytes.len() as i64,
            truncated: false,
            truncated_reason: None,
        });
    }

    let semaphore = state.proxy_raw_async_semaphore.clone();
    let invoke_id = invoke_id.to_string();
    let kind_for_spool = kind;
    let bytes_for_spool = bytes.clone();
    if semaphore.available_permits() == 0 {
        let codec = state.config.proxy_raw_compression;
        let spool = match RawOverflowSpool::create(state, &invoke_id, kind_for_spool, codec) {
            Ok(spool) => spool,
            Err(err) => {
                warn!(
                    capture_path = "capture_unavailable",
                    capture_unavailable_reason = "spool_capacity",
                    error = %err,
                    "raw capture unavailable because the durable spool cannot accept it"
                );
                return PendingRawPayloadWrite::Ready(RawPayloadMeta {
                    path: None,
                    size_bytes: bytes_for_spool.len() as i64,
                    truncated: true,
                    truncated_reason: Some("capture_unavailable:spool_capacity".to_string()),
                });
            }
        };
        return PendingRawPayloadWrite::Task(tokio::spawn(async move {
            let mut spool = spool;
            if let Err(err) = spool.append(&bytes_for_spool) {
                return RawPayloadMeta {
                    path: None,
                    size_bytes: bytes_for_spool.len() as i64,
                    truncated: true,
                    truncated_reason: Some(if err.to_string().contains("capacity") {
                        "capture_unavailable:spool_capacity".to_string()
                    } else {
                        "capture_unavailable:spool_write_failed".to_string()
                    }),
                };
            }
            spool.finish(bytes_for_spool.len() as i64).await
        }));
    }

    let config = state.config.clone();
    PendingRawPayloadWrite::Task(tokio::spawn(async move {
        // Queue behind the bounded CPU writer pool instead of dropping an enabled capture.
        let permit = semaphore
            .acquire_owned()
            .await
            .expect("raw writer semaphore is live");
        let _permit = permit;
        store_raw_payload_file(&config, &invoke_id, kind, bytes).await
    }))
}

pub(crate) fn spawn_raw_payload_snapshot_write(
    state: Arc<AppState>,
    invoke_id: &str,
    kind: &'static str,
    snapshot: PoolReplayBodySnapshot,
    enabled: bool,
) -> PendingRawPayloadWrite {
    match snapshot {
        PoolReplayBodySnapshot::Empty => PendingRawPayloadWrite::Ready(RawPayloadMeta::default()),
        PoolReplayBodySnapshot::Memory(bytes) => {
            spawn_raw_payload_file_write(state.as_ref(), invoke_id, kind, bytes, enabled)
        }
        PoolReplayBodySnapshot::File { size, .. } if !enabled => {
            PendingRawPayloadWrite::Ready(RawPayloadMeta {
                path: None,
                size_bytes: size as i64,
                truncated: false,
                truncated_reason: None,
            })
        }
        PoolReplayBodySnapshot::File { temp_file, size } => {
            let config = state.config.clone();
            let semaphore = state.proxy_raw_async_semaphore.clone();
            let invoke_id = invoke_id.to_string();
            let source_path = temp_file.path.clone();
            PendingRawPayloadWrite::Task(tokio::spawn(async move {
                let _temp_file_guard = temp_file;
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("raw writer semaphore is live");
                store_raw_payload_snapshot_file(&config, &invoke_id, kind, source_path, size).await
            }))
        }
    }
}
