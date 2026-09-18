pub(crate) struct RunningProxyCaptureRecordRequest<'a>(
    pub(crate) &'a str,
    pub(crate) &'a str,
    pub(crate) ProxyCaptureTarget,
    pub(crate) &'a RequestCaptureInfo,
    pub(crate) Option<&'a str>,
    pub(crate) Option<&'a str>,
    pub(crate) Option<&'a str>,
    pub(crate) bool,
    pub(crate) Option<i64>,
    pub(crate) Option<&'a str>,
    pub(crate) Option<&'a str>,
    pub(crate) Option<&'a str>,
    pub(crate) Option<&'a str>,
    pub(crate) Option<usize>,
    pub(crate) Option<usize>,
    pub(crate) Option<&'a str>,
    pub(crate) Option<&'a str>,
    pub(crate) f64,
    pub(crate) f64,
    pub(crate) f64,
    pub(crate) f64,
);

pub(crate) fn build_running_proxy_capture_record(
    request: RunningProxyCaptureRecordRequest<'_>,
) -> ProxyCaptureRecord {
    let payload = build_running_proxy_capture_payload(&request);
    let RunningProxyCaptureRecordRequest(
        invoke_id,
        occurred_at,
        _target,
        request_info,
        _requester_ip,
        _sticky_key,
        _prompt_cache_key,
        _pool_route_active,
        _upstream_account_id,
        _upstream_account_name,
        _upstream_account_kind,
        _upstream_base_url_host,
        _proxy_display_name,
        _pool_attempt_count,
        _pool_distinct_account_count,
        _pool_attempt_terminal_reason,
        _response_content_encoding,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        t_upstream_ttfb_ms,
    ) = request;
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
        payload: Some(payload),
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

struct RunningProxyCapturePayloadFields<'a> {
    target: ProxyCaptureTarget,
    request_info: &'a RequestCaptureInfo,
    requester_ip: Option<&'a str>,
    sticky_key: Option<&'a str>,
    prompt_cache_key: Option<&'a str>,
    pool_route_active: bool,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&'a str>,
    upstream_account_kind: Option<&'a str>,
    upstream_base_url_host: Option<&'a str>,
    proxy_display_name: Option<&'a str>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&'a str>,
    response_content_encoding: Option<&'a str>,
}

fn build_running_proxy_capture_payload(request: &RunningProxyCaptureRecordRequest<'_>) -> String {
    let RunningProxyCaptureRecordRequest(
        _invoke_id,
        _occurred_at,
        target,
        request_info,
        requester_ip,
        sticky_key,
        prompt_cache_key,
        pool_route_active,
        upstream_account_id,
        upstream_account_name,
        upstream_account_kind,
        upstream_base_url_host,
        proxy_display_name,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
        response_content_encoding,
        _t_req_read_ms,
        _t_req_parse_ms,
        _t_upstream_connect_ms,
        _t_upstream_ttfb_ms,
    ) = *request;
    build_running_proxy_capture_payload_summary(RunningProxyCapturePayloadFields {
        target,
        request_info,
        requester_ip,
        sticky_key,
        prompt_cache_key,
        pool_route_active,
        upstream_account_id,
        upstream_account_name,
        upstream_account_kind,
        upstream_base_url_host,
        proxy_display_name,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
        response_content_encoding,
    })
}

fn build_running_proxy_capture_payload_summary(
    fields: RunningProxyCapturePayloadFields<'_>,
) -> String {
    build_proxy_payload_summary(ProxyPayloadSummary {
        target: fields.target,
        status: StatusCode::OK,
        is_stream: fields.request_info.is_stream,
        request_contains_encrypted_content: fields.request_info.contains_encrypted_content,
        response_contains_encrypted_content: false,
        compaction_request_kind: fields.request_info.compaction_request_kind,
        compaction_response_kind: None,
        image_intent: fields.request_info.image_intent.as_deref(),
        request_model: fields.request_info.model.as_deref(),
        requested_service_tier: fields.request_info.requested_service_tier.as_deref(),
        billing_service_tier: None,
        reasoning_effort: fields.request_info.reasoning_effort.as_deref(),
        response_model: None,
        usage_missing_reason: None,
        request_parse_error: fields.request_info.parse_error.as_deref(),
        request_compression_algorithm: None,
        request_compression_mode: None,
        request_compression_logical_body_bytes: None,
        request_compression_transmitted_body_bytes: None,
        request_compression_transmission_complete: None,
        failure_kind: None,
        requester_ip: fields.requester_ip,
        request_user_agent: None,
        request_x_forwarded_for: None,
        request_forwarded: None,
        request_x_real_ip: None,
        upstream_scope: if fields.pool_route_active {
            INVOCATION_UPSTREAM_SCOPE_INTERNAL
        } else {
            INVOCATION_UPSTREAM_SCOPE_EXTERNAL
        },
        route_mode: if fields.pool_route_active {
            INVOCATION_ROUTE_MODE_POOL
        } else {
            INVOCATION_ROUTE_MODE_FORWARD_PROXY
        },
        sticky_key: fields.sticky_key,
        prompt_cache_key: fields.prompt_cache_key,
        prompt_cache_key_attribution_source: fields
            .request_info
            .prompt_cache_key_attribution_source
            .as_deref(),
        client_fingerprint: None,
        client_header_fingerprints: None,
        upstream_account_id: fields.upstream_account_id,
        upstream_account_name: fields.upstream_account_name,
        upstream_account_kind: fields.upstream_account_kind,
        upstream_base_url_host: fields.upstream_base_url_host,
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
        response_content_encoding: fields.response_content_encoding,
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
        proxy_display_name: fields.proxy_display_name,
        proxy_weight_delta: None,
        pool_attempt_count: fields.pool_attempt_count,
        pool_distinct_account_count: fields.pool_distinct_account_count,
        pool_attempt_terminal_reason: fields.pool_attempt_terminal_reason,
        blocked_binding: None,
    })
}
fn remove_raw_overflow_spool_segments(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

pub(crate) async fn recover_raw_overflow_spools(config: &AppConfig) {
    let directory = config.resolved_proxy_raw_dir().join(RAW_OVERFLOW_SPOOL_DIR);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return,
        Err(err) => {
            warn!(path = %directory.display(), error = %err, "failed to scan raw overflow spool directory");
            return;
        }
    };

    let mut captures = HashMap::<String, Vec<(PathBuf, RawOverflowSpoolHeader)>>::new();
    let mut corrupt_captures = std::collections::HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("frames") {
            continue;
        }
        let header = match run_blocking_raw_writer_io({
            let path = path.clone();
            move || read_raw_overflow_spool_segment(&path).map(|(header, _)| header)
        })
        .await
        {
            Ok(header) => header,
            Err(err) => {
                warn!(path = %path.display(), error = %err, "raw overflow spool is incomplete or corrupt; retaining for inspection");
                if let Some(capture_key) = raw_overflow_spool_capture_key_from_path(&path) {
                    corrupt_captures.insert(capture_key);
                }
                continue;
            }
        };
        let capture_key = raw_overflow_spool_capture_key(&path, &header);
        captures
            .entry(capture_key)
            .or_default()
            .push((path, header));
    }

    let semaphore = Arc::new(Semaphore::new(proxy_raw_async_writer_limit(config)));
    for (capture_key, mut segments) in captures {
        if corrupt_captures.contains(&capture_key) {
            warn!(
                capture_key,
                spool_segment_count = segments.len(),
                "raw overflow spool capture has a corrupt segment; retaining all segments"
            );
            continue;
        }
        segments.sort_by_key(|(_, header)| header.segment_index);
        let header = match validate_raw_overflow_spool_segments(&segments) {
            Ok(header) => header,
            Err(err) => {
                warn!(error = %err, spool_segment_count = segments.len(), "raw overflow spool capture is incomplete or inconsistent; retaining for inspection");
                continue;
            }
        };
        let paths = segments
            .into_iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>();
        let meta =
            replay_raw_overflow_spool_segments(config, semaphore.clone(), paths.clone()).await;
        if meta.path.is_some() || meta.truncated_reason.as_deref() == Some("max_bytes_exceeded") {
            remove_raw_overflow_spool_segments(&paths);
            info!(
                invoke_id = %header.invoke_id,
                kind = %header.kind,
                replay_count = 1,
                spool_segment_count = paths.len(),
                "recovered raw overflow spool"
            );
        } else {
            warn!(
                invoke_id = %header.invoke_id,
                kind = %header.kind,
                spool_segment_count = paths.len(),
                reason = ?meta.truncated_reason,
                "raw overflow spool recovery did not publish a payload; retaining spool"
            );
        }
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

pub(crate) struct AsyncStreamingRawPayloadWriter {
    tx: Option<std::sync::mpsc::SyncSender<TrackedRawPayloadChunk>>,
    meta_rx: Option<oneshot::Receiver<RawPayloadMeta>>,
    observed_size_bytes: i64,
    local_truncated_reason: Option<String>,
    local_truncated: bool,
    spool: Option<RawOverflowSpool>,
}

static RAW_ASYNC_WRITER_QUEUED_BYTES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn proxy_raw_async_writer_queued_bytes() -> usize {
    RAW_ASYNC_WRITER_QUEUED_BYTES.load(std::sync::atomic::Ordering::Relaxed)
}

struct TrackedRawPayloadChunk {
    bytes: Option<Bytes>,
}

impl TrackedRawPayloadChunk {
    fn new(bytes: Bytes) -> Self {
        Self { bytes: Some(bytes) }
    }

    fn into_bytes(mut self) -> Bytes {
        let bytes = self.bytes.take().unwrap_or_default();
        release_raw_async_writer_queued_bytes(bytes.len());
        bytes
    }
}

impl Drop for TrackedRawPayloadChunk {
    fn drop(&mut self) {
        if let Some(bytes) = self.bytes.as_ref() {
            release_raw_async_writer_queued_bytes(bytes.len());
        }
    }
}

trait RawPayloadChunk {
    fn into_bytes(self) -> Bytes;
}

impl RawPayloadChunk for Bytes {
    fn into_bytes(self) -> Bytes {
        self
    }
}

impl RawPayloadChunk for TrackedRawPayloadChunk {
    fn into_bytes(self) -> Bytes {
        TrackedRawPayloadChunk::into_bytes(self)
    }
}

impl AsyncStreamingRawPayloadWriter {
    pub(crate) fn new(
        state: &AppState,
        invoke_id: &str,
        kind: &'static str,
        enabled: bool,
        wire_content_encoding: Option<&str>,
    ) -> Self {
        if !enabled {
            return Self::disabled();
        }

        let path = raw_payload_path_for_kind(
            &state.config.resolved_proxy_raw_dir(),
            invoke_id,
            kind,
            false,
        );
        let max_bytes = state.config.proxy_raw_max_bytes;
        let immediate_gzip_bytes = state.config.proxy_raw_immediate_compression_threshold();
        // A wire-compressed response is already compressed. Preserve its bytes so the
        // capture path never spends CPU decoding and recompressing gzip/deflate/zstd.
        let codec = wire_content_encoding
            .map(str::trim)
            .filter(|encoding| !encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity"))
            .map(|_| RawCompressionCodec::None)
            .unwrap_or(state.config.proxy_raw_compression);
        let semaphore = state.proxy_raw_async_semaphore.clone();
        let writer_max = proxy_raw_async_writer_limit(&state.config);
        let writer_active = writer_max.saturating_sub(semaphore.available_permits());
        let permit = semaphore.clone().try_acquire_owned();
        if permit.is_err() {
            return Self::from_overflow_spool(
                state,
                invoke_id,
                kind,
                codec,
                writer_active,
                writer_max,
            );
        }
        let permit = permit.expect("checked above");
        Self::spawn_direct_writer(path, max_bytes, immediate_gzip_bytes, codec, permit)
    }

    fn disabled() -> Self {
        Self {
            tx: None,
            meta_rx: None,
            observed_size_bytes: 0,
            local_truncated_reason: None,
            local_truncated: false,
            spool: None,
        }
    }

    fn from_overflow_spool(
        state: &AppState,
        invoke_id: &str,
        kind: &'static str,
        codec: RawCompressionCodec,
        writer_active: usize,
        writer_max: usize,
    ) -> Self {
        match RawOverflowSpool::create(state, invoke_id, kind, codec) {
            Ok(spool) => {
                debug!(
                    capture_path = "overflow_spool",
                    storage_codec = ?codec,
                    writer_active,
                    writer_max,
                    spool_pending_records = 0,
                    spool_pending_bytes = 0,
                    "raw capture queued to durable overflow spool"
                );
                Self {
                    spool: Some(spool),
                    ..Self::disabled()
                }
            }
            Err(err) => {
                warn!(
                    capture_path = "capture_unavailable",
                    capture_unavailable_reason = "spool_capacity",
                    storage_codec = ?codec,
                    writer_active,
                    writer_max,
                    error = %err,
                    "raw streaming capture unavailable because the durable spool cannot accept it"
                );
                Self {
                    local_truncated_reason: Some("capture_unavailable:spool_capacity".to_string()),
                    local_truncated: true,
                    ..Self::disabled()
                }
            }
        }
    }

    fn spawn_direct_writer(
        path: PathBuf,
        max_bytes: Option<usize>,
        immediate_gzip_bytes: Option<usize>,
        codec: RawCompressionCodec,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<TrackedRawPayloadChunk>(64);
        let (meta_tx, meta_rx) = oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let meta = write_direct_streaming_raw_payload_to_file_tracked(
                path,
                max_bytes,
                immediate_gzip_bytes,
                codec,
                rx,
            );
            let _ = meta_tx.send(meta);
        });

        Self {
            tx: Some(tx),
            meta_rx: Some(meta_rx),
            observed_size_bytes: 0,
            local_truncated_reason: None,
            local_truncated: false,
            spool: None,
        }
    }

    fn mark_writer_closed(&mut self, message: String) {
        self.local_truncated = true;
        self.local_truncated_reason.get_or_insert_with(|| {
            if message.starts_with("capture_unavailable:") {
                message
            } else {
                format!("write_failed:{message}")
            }
        });
        self.tx = None;
    }

    pub(crate) fn append(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.observed_size_bytes = self.observed_size_bytes.saturating_add(bytes.len() as i64);
        if let Some(spool) = self.spool.as_mut() {
            if let Err(err) = spool.append(bytes) {
                self.spool = None;
                let reason = if err.to_string().contains("capacity") {
                    "capture_unavailable:spool_capacity"
                } else {
                    "capture_unavailable:spool_write_failed"
                };
                self.mark_writer_closed(reason.to_string());
            }
            return;
        }
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        RAW_ASYNC_WRITER_QUEUED_BYTES.fetch_add(bytes.len(), std::sync::atomic::Ordering::Relaxed);
        match tx.try_send(TrackedRawPayloadChunk::new(Bytes::copy_from_slice(bytes))) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                self.mark_writer_closed("capture_unavailable:ingress_queue_full".to_string());
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.mark_writer_closed("capture_unavailable:writer_closed".to_string());
            }
        }
    }

    pub(crate) async fn finish(mut self) -> RawPayloadMeta {
        self.tx.take();
        let mut meta = if let Some(spool) = self.spool.take() {
            spool.finish(self.observed_size_bytes).await
        } else {
            match self.meta_rx.take() {
                Some(meta_rx) => match meta_rx.await {
                    Ok(meta) => meta,
                    Err(err) => RawPayloadMeta {
                        path: None,
                        size_bytes: self.observed_size_bytes,
                        truncated: true,
                        truncated_reason: Some(format!("write_failed:{err}")),
                    },
                },
                None => RawPayloadMeta::default(),
            }
        };
        meta.size_bytes = self.observed_size_bytes;
        if self.local_truncated {
            meta.truncated = true;
            if meta.truncated_reason.is_none() {
                meta.truncated_reason = self.local_truncated_reason.clone();
            }
            if self
                .local_truncated_reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("capture_unavailable:"))
                && let Some(path) = meta.path.take()
            {
                let _ = fs::remove_file(path);
            }
        }
        meta
    }
}
