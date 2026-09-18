use super::*;
pub(crate) async fn persist_and_broadcast_proxy_capture(
    state: &AppState,
    capture_started: Instant,
    mut record: ProxyCaptureRecord,
) -> Result<()> {
    let enqueue_started = Instant::now();
    if !record.timings.t_total_ms.is_finite() || record.timings.t_total_ms <= 0.0 {
        record.timings.t_total_ms = elapsed_ms(capture_started);
    }
    let inserted_record = api_invocation_from_runtime_record(&record);
    let invoke_id = inserted_record.invoke_id.clone();
    let duplicate_terminal = remove_proxy_runtime_snapshot_for_terminal(state, &inserted_record);
    if duplicate_terminal {
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %inserted_record.occurred_at,
            business_unblocked_record_write = true,
            "duplicate raw proxy capture record skipped before sqlite enqueue"
        );
        schedule_proxy_capture_follow_up_after_terminal_enqueue(
            state,
            &invoke_id,
            "duplicate_raw_terminal",
        );
        return Ok(());
    }
    super::usage_persistence::observe_successful_proxy_capture_model_route_cache(state, &record)
        .await;
    let projection = register_terminal_projection_before_enqueue(state, &inserted_record).await;
    let delta = &projection.dashboard;
    let startup_backfill_tasks = startup_backfill_tasks_for_terminal(&inserted_record);
    let terminal_enqueue = enqueue_raw_capture_terminal(
        state,
        record,
        capture_started,
        delta,
        projection.event_id.into_iter().collect(),
        startup_backfill_tasks,
    );
    let terminal_enqueued = terminal_enqueue.enqueued;
    if !terminal_enqueued {
        rollback_terminal_projection_before_enqueue(state, &inserted_record, &projection).await;
        let terminal_tombstone_cleared = state
            .proxy_runtime_invocations
            .clear_terminal_tombstone(&inserted_record.invoke_id, &inserted_record.occurred_at);
        warn!(
            invoke_id = %invoke_id,
            occurred_at = %inserted_record.occurred_at,
            enqueue_failed_by_class = "raw_terminal_invocation",
            terminal_tombstone_cleared,
            durability_mode = terminal_enqueue.durability_mode.as_str(),
            journal_sequence = ?terminal_enqueue.journal_sequence,
            business_unblocked_record_write = true,
            record_flush_deferred_or_failed = "raw_terminal_invocation_enqueue_failed",
            "raw proxy capture record dropped by sqlite write controller"
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
            record_flush_deferred_or_failed = "raw_terminal_invocation_enqueued_async",
            "raw proxy capture record queued for sqlite write controller"
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
                &inserted_record,
                RuntimeMutationKind::TerminalCommitted,
            ));
        #[cfg(test)]
        if state.broadcaster.receiver_count() > 0 {
            let _ = state.broadcaster.send(BroadcastPayload::Records {
                records: vec![inserted_record.clone()],
            });
        }
    }
    if terminal_enqueued {
        schedule_dashboard_activity_live_snapshot(state);
        schedule_proxy_capture_follow_up_after_terminal_enqueue(state, &invoke_id, "raw_terminal");
    }
    Ok(())
}

fn enqueue_raw_capture_terminal(
    state: &AppState,
    record: ProxyCaptureRecord,
    capture_started: Instant,
    delta: &DashboardActivityTerminalDeltaOutcome,
    terminal_projection_event_ids: Vec<u64>,
    startup_backfill_tasks: Vec<StartupBackfillTask>,
) -> TerminalEnqueueOutcome {
    state
        .sqlite_batch_writer
        .enqueue_terminal(BatchedTerminalInvocationWrite {
            record,
            capture_started: Some(capture_started),
            raw_capture: true,
            dashboard_terminal_sequence: delta.terminal_sequence,
            terminal_projection_event_ids,
            startup_backfill_tasks,
        })
}

pub(crate) async fn persist_proxy_capture_record(
    pool: &Pool<Sqlite>,
    capture_started: Instant,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    persist_proxy_capture_record_core(pool, capture_started, record, true).await
}

pub(crate) async fn persist_proxy_capture_record_core(
    pool: &Pool<Sqlite>,
    capture_started: Instant,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
) -> Result<Option<ApiInvocation>> {
    let mut tx = pool.begin().await?;
    let persisted =
        persist_proxy_capture_record_tx(tx.as_mut(), capture_started, record, write_derived_inline)
            .await?;
    tx.commit().await?;
    Ok(persisted)
}

pub(crate) async fn persist_proxy_capture_record_tx(
    tx: &mut SqliteConnection,
    capture_started: Instant,
    mut record: ProxyCaptureRecord,
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
    let persist_started = Instant::now();
    let created_at = format_utc_iso_millis(Utc::now());
    if !record.timings.t_total_ms.is_finite() || record.timings.t_total_ms <= 0.0 {
        record.timings.t_total_ms = elapsed_ms(capture_started);
    }
    let context = ProxyCaptureWriteContext {
        record: &record,
        raw_response: &raw_response,
        resp_raw: &resp_raw,
        failure_kind: failure_kind.as_deref(),
        failure: &failure,
        t_persist_ms: nullable_runtime_timing_value(record.timings.t_persist_ms),
        created_at: &created_at,
    };
    let Some((invocation_id, core_write_path)) =
        persist_proxy_capture_identity(tx, context).await?
    else {
        return Ok(None);
    };
    if write_derived_inline {
        touch_invocation_upstream_account_last_activity_tx(
            &mut *tx,
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
        recompute_invocation_hourly_rollups_for_ids_tx(&mut *tx, &[invocation_id]).await?;
        save_hourly_rollup_live_progress_tx(
            &mut *tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            invocation_id,
        )
        .await?;
    }
    let measured_t_persist_ms = elapsed_ms(persist_started);
    sqlx::query("UPDATE codex_invocations SET t_persist_ms = ?2 WHERE id = ?1")
        .bind(invocation_id)
        .bind(measured_t_persist_ms)
        .execute(&mut *tx)
        .await?;
    let persisted =
        load_persisted_api_invocation_tx(&mut *tx, &record.invoke_id, &record.occurred_at).await?;

    let core_write_elapsed_ms = persist_started.elapsed().as_millis() as u64;
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
            "proxy capture raw core invocation write was slow"
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
            "proxy capture raw core invocation write completed"
        );
    }
    Ok(Some(persisted))
}

struct ProxyCaptureWriteContext<'a> {
    record: &'a ProxyCaptureRecord,
    raw_response: &'a str,
    resp_raw: &'a RawPayloadMeta,
    failure_kind: Option<&'a str>,
    failure: &'a FailureClassification,
    t_persist_ms: Option<f64>,
    created_at: &'a str,
}

async fn persist_proxy_capture_identity(
    tx: &mut SqliteConnection,
    context: ProxyCaptureWriteContext<'_>,
) -> Result<Option<(i64, &'static str)>> {
    let existing = load_persisted_invocation_identity_tx(
        &mut *tx,
        &context.record.invoke_id,
        &context.record.occurred_at,
    )
    .await?;
    if let Some(existing) = existing {
        if !update_existing_proxy_capture_if_allowed(
            tx,
            existing.id,
            existing.status.as_deref(),
            existing.failure_kind.as_deref(),
            &context,
        )
        .await?
        {
            return Ok(None);
        }
        return Ok(Some((existing.id, "update_existing")));
    }

    if let Some(id) = insert_proxy_capture_row(&mut *tx, &context).await? {
        return Ok(Some((id, "insert_missing")));
    }

    let Some(existing) = load_persisted_invocation_identity_tx(
        &mut *tx,
        &context.record.invoke_id,
        &context.record.occurred_at,
    )
    .await?
    else {
        return Ok(None);
    };
    if !update_existing_proxy_capture_if_allowed(
        tx,
        existing.id,
        existing.status.as_deref(),
        existing.failure_kind.as_deref(),
        &context,
    )
    .await?
    {
        return Ok(None);
    }
    Ok(Some((existing.id, "update_race")))
}

async fn update_existing_proxy_capture_if_allowed(
    tx: &mut SqliteConnection,
    id: i64,
    existing_status: Option<&str>,
    existing_failure_kind: Option<&str>,
    context: &ProxyCaptureWriteContext<'_>,
) -> Result<bool> {
    let record = context.record;
    if !persisted_invocation_allows_proxy_record_update(
        existing_status,
        existing_failure_kind,
        &record.status,
    ) {
        return Ok(false);
    }
    update_existing_proxy_invocation_record_tx(
        tx,
        ProxyInvocationUpdateRequest {
            id,
            record,
            raw_response: context.raw_response,
            resp_raw: context.resp_raw,
            failure_kind: context.failure_kind,
            failure_class: context.failure.failure_class.as_str(),
            is_actionable: context.failure.is_actionable,
            t_total_ms: Some(record.timings.t_total_ms),
            t_req_read_ms: Some(record.timings.t_req_read_ms),
            t_req_parse_ms: Some(record.timings.t_req_parse_ms),
            t_upstream_connect_ms: Some(record.timings.t_upstream_connect_ms),
            t_upstream_ttfb_ms: Some(record.timings.t_upstream_ttfb_ms),
            first_token_ms: record.timings.first_token_ms,
            t_upstream_stream_ms: Some(record.timings.t_upstream_stream_ms),
            t_resp_parse_ms: Some(record.timings.t_resp_parse_ms),
            t_persist_ms: context.t_persist_ms,
        },
    )
    .await
}

async fn insert_proxy_capture_row(
    tx: &mut SqliteConnection,
    context: &ProxyCaptureWriteContext<'_>,
) -> Result<Option<i64>> {
    let record = context.record;
    let result = sqlx::query(PROXY_CAPTURE_INSERT_QUERY)
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
        .bind(context.failure_kind)
        .bind(context.failure.failure_class.as_str())
        .bind(context.failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(context.raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&record.req_raw))
        .bind(record.req_raw.size_bytes)
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(context.resp_raw.path.as_deref())
        .bind(raw_payload_meta_codec(context.resp_raw))
        .bind(context.resp_raw.size_bytes)
        .bind(context.resp_raw.truncated as i64)
        .bind(context.resp_raw.truncated_reason.as_deref())
        .bind(record.timings.t_total_ms)
        .bind(record.timings.t_req_read_ms)
        .bind(record.timings.t_req_parse_ms)
        .bind(record.timings.t_upstream_connect_ms)
        .bind(record.timings.t_upstream_ttfb_ms)
        .bind(record.timings.first_token_ms)
        .bind(record.timings.t_upstream_stream_ms)
        .bind(record.timings.t_resp_parse_ms)
        .bind(context.t_persist_ms)
        .bind(context.created_at)
        .execute(&mut *tx)
        .await?;
    Ok((result.rows_affected() > 0).then(|| result.last_insert_rowid()))
}

const PROXY_CAPTURE_INSERT_QUERY: &str = r#"
INSERT OR IGNORE INTO codex_invocations (
    invoke_id, occurred_at, source, model, input_tokens, output_tokens,
    cache_input_tokens, reasoning_tokens, total_tokens, cost, cost_input,
    cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
    cost_estimated, price_version, status, error_message, failure_kind,
    failure_class, is_actionable, payload, raw_response, request_raw_path,
    request_raw_codec, request_raw_size, request_raw_truncated,
    request_raw_truncated_reason, response_raw_path, response_raw_codec,
    response_raw_size, response_raw_truncated, response_raw_truncated_reason,
    t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
    t_upstream_ttfb_ms, first_token_ms, t_upstream_stream_ms, t_resp_parse_ms,
    t_persist_ms, created_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
    ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28,
    ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41,
    ?42, ?43, ?44
)
"#;

pub(crate) fn read_proxy_raw_bytes(
    path: &str,
    fallback_root: Option<&Path>,
) -> io::Result<Vec<u8>> {
    let mut last_error = None;
    for candidate in resolved_raw_path_read_candidates(path, fallback_root) {
        match fs::read(&candidate) {
            Ok(content) => return decode_proxy_raw_file_bytes(&candidate, content),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                last_error = Some(err);
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("raw payload file not found for path {path}"),
        )
    }))
}

pub(crate) fn decode_proxy_raw_file_bytes(path: &Path, bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"))
    {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to decompress raw payload {}: {err}", path.display()),
            )
        })?;
        Ok(decoded)
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zst"))
    {
        zstd::stream::decode_all(bytes.as_slice()).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to decompress raw payload {}: {err}", path.display()),
            )
        })
    } else {
        Ok(bytes)
    }
}
