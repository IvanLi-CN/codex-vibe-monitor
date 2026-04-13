pub(crate) fn deflate_stream_uses_zlib_wrapper(header: &[u8]) -> bool {
    if header.len() < 2 {
        return true;
    }

    let cmf = header[0];
    let flg = header[1];
    let method = cmf & 0x0f;
    let window_bits = cmf >> 4;
    let header_word = (u16::from(cmf) << 8) | u16::from(flg);
    method == 8 && window_bits <= 7 && header_word % 31 == 0
}

#[allow(dead_code)]
pub(crate) fn wrap_decoded_response_reader(
    mut reader: Box<dyn Read + Send>,
    content_encoding: Option<&str>,
) -> std::result::Result<Box<dyn Read + Send>, String> {
    let encodings = parse_content_encodings(content_encoding);
    for encoding in encodings.iter().rev() {
        reader = match encoding.as_str() {
            "identity" => reader,
            "gzip" | "x-gzip" => Box::new(GzDecoder::new(reader)),
            "br" => Box::new(BrotliDecompressor::new(reader, 4096)),
            "deflate" => {
                let mut buffered = io::BufReader::new(reader);
                let header = buffered.fill_buf().map_err(|err| err.to_string())?;
                if deflate_stream_uses_zlib_wrapper(header) {
                    Box::new(ZlibDecoder::new(buffered))
                } else {
                    Box::new(DeflateDecoder::new(buffered))
                }
            }
            other => return Err(format!("unsupported_content_encoding:{other}")),
        };
    }
    Ok(reader)
}

#[allow(dead_code)]
pub(crate) fn open_decoded_response_reader(
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<Box<dyn Read + Send>, String> {
    let file = fs::File::open(path).map_err(|err| err.to_string())?;
    wrap_decoded_response_reader(Box::new(file), content_encoding)
}

#[allow(dead_code)]
pub(crate) fn parse_nonstream_response_payload_from_raw_file(
    target: ProxyCaptureTarget,
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<ResponseCaptureInfo, String> {
    let mut reader = open_decoded_response_reader(path, content_encoding)?;
    let mut decoded = Vec::new();
    reader
        .by_ref()
        .take((BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    if decoded.len() > BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES {
        decoded.truncate(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES);
        let mut response_info = parse_target_response_payload(target, &decoded, false, None);
        merge_response_capture_reason(
            &mut response_info,
            PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
        );
        return Ok(response_info);
    }
    Ok(parse_target_response_payload(target, &decoded, false, None))
}

#[allow(dead_code)]
pub(crate) fn parse_target_response_payload_from_raw_file(
    target: ProxyCaptureTarget,
    path: &Path,
    is_stream_hint: bool,
    content_encoding: Option<&str>,
) -> std::result::Result<ResponseCaptureInfo, String> {
    if is_stream_hint {
        let reader = open_decoded_response_reader(path, content_encoding)?;
        parse_stream_response_payload_from_reader(reader).map_err(|err| err.to_string())
    } else {
        parse_nonstream_response_payload_from_raw_file(target, path, content_encoding)
    }
}

#[allow(dead_code)]
pub(crate) fn parse_target_response_payload_from_capture(
    target: ProxyCaptureTarget,
    resp_raw: &RawPayloadMeta,
    preview_bytes: &[u8],
    is_stream_hint: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    #[cfg(test)]
    RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);

    if let Some(path) = resp_raw.path.as_deref() {
        let path = PathBuf::from(path);
        match parse_target_response_payload_from_raw_file(
            target,
            &path,
            is_stream_hint,
            content_encoding,
        ) {
            Ok(response_info) => response_info,
            Err(reason) => {
                let mut response_info = parse_target_response_payload(
                    target,
                    preview_bytes,
                    is_stream_hint,
                    content_encoding,
                );
                merge_response_capture_reason(&mut response_info, reason);
                response_info
            }
        }
    } else {
        parse_target_response_payload(target, preview_bytes, is_stream_hint, content_encoding)
    }
}

pub(crate) fn summarize_pool_upstream_http_failure(
    status: StatusCode,
    upstream_request_id_header: Option<&str>,
    bytes: &[u8],
) -> (Option<String>, Option<String>, Option<String>, String) {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        let detail = summarize_plaintext_upstream_error(bytes);
        let message = detail.as_deref().map_or_else(
            || format!("pool upstream responded with {}", status.as_u16()),
            |detail| {
                format!(
                    "pool upstream responded with {}: {}",
                    status.as_u16(),
                    detail
                )
            },
        );
        return (
            None,
            detail,
            upstream_request_id_header.map(|value| value.to_string()),
            message,
        );
    };
    let upstream_error_code = extract_upstream_error_code(&value);
    let upstream_error_message = extract_upstream_error_message(&value);
    let upstream_request_id = upstream_request_id_header
        .map(|value| value.to_string())
        .or_else(|| extract_upstream_request_id(&value));

    let detail = upstream_error_message
        .as_deref()
        .or_else(|| value.get("message").and_then(|entry| entry.as_str()))
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
        .map(|detail| detail.chars().take(240).collect::<String>());

    let message = if let Some(detail) = detail {
        format!(
            "pool upstream responded with {}: {}",
            status.as_u16(),
            detail
        )
    } else {
        format!("pool upstream responded with {}", status.as_u16())
    };

    (
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
        message,
    )
}

pub(crate) struct NormalizedPoolFailureRecord {
    attempt_status: &'static str,
    upstream_http_status: Option<StatusCode>,
    downstream_http_status: Option<StatusCode>,
    canonical_error_message: String,
    downstream_error_message: Option<String>,
}

pub(crate) fn default_oauth_transport_failure_message(failure_kind: &'static str) -> &'static str {
    match failure_kind {
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM => "failed to contact oauth codex upstream",
        PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT => "oauth codex upstream handshake timed out",
        PROXY_FAILURE_UPSTREAM_STREAM_ERROR => "oauth codex upstream stream error",
        _ => "oauth codex upstream transport failure",
    }
}

pub(crate) fn normalize_pool_upstream_failure_record(
    status: StatusCode,
    oauth_transport_failure_kind: Option<&'static str>,
    message: &str,
    upstream_error_message: Option<&str>,
) -> NormalizedPoolFailureRecord {
    if let Some(failure_kind) = oauth_transport_failure_kind {
        let wrapped_prefix = format!("pool upstream responded with {}:", status.as_u16());
        let canonical_error_message = upstream_error_message
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                message
                    .strip_prefix(&wrapped_prefix)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| default_oauth_transport_failure_message(failure_kind))
            .to_string();
        return NormalizedPoolFailureRecord {
            attempt_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            upstream_http_status: None,
            downstream_http_status: Some(status),
            canonical_error_message,
            downstream_error_message: Some(message.to_string()),
        };
    }

    NormalizedPoolFailureRecord {
        attempt_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
        upstream_http_status: Some(status),
        downstream_http_status: None,
        canonical_error_message: message.to_string(),
        downstream_error_message: None,
    }
}

pub(crate) async fn estimate_proxy_cost_from_shared_catalog(
    catalog: &Arc<RwLock<PricingCatalog>>,
    model: Option<&str>,
    usage: &ParsedUsage,
    billing_service_tier: Option<&str>,
    pricing_mode: ProxyPricingMode,
) -> (Option<f64>, bool, Option<String>) {
    let guard = catalog.read().await;
    estimate_proxy_cost(&guard, model, usage, billing_service_tier, pricing_mode)
}

pub(crate) fn has_billable_usage(usage: &ParsedUsage) -> bool {
    usage.input_tokens.unwrap_or(0).max(0) > 0
        || usage.output_tokens.unwrap_or(0).max(0) > 0
        || usage.cache_input_tokens.unwrap_or(0).max(0) > 0
        || usage.reasoning_tokens.unwrap_or(0).max(0) > 0
}

pub(crate) fn resolve_pricing_for_model<'a>(
    catalog: &'a PricingCatalog,
    model: &str,
) -> Option<&'a ModelPricing> {
    if let Some(pricing) = catalog.models.get(model) {
        return Some(pricing);
    }
    dated_model_alias_base(model).and_then(|base| catalog.models.get(base))
}

pub(crate) fn dated_model_alias_base(model: &str) -> Option<&str> {
    const DATED_SUFFIX_LEN: usize = 11; // -YYYY-MM-DD
    if model.len() <= DATED_SUFFIX_LEN {
        return None;
    }
    let suffix = &model.as_bytes()[model.len() - DATED_SUFFIX_LEN..];
    let is_dated_suffix = suffix[0] == b'-'
        && suffix[1].is_ascii_digit()
        && suffix[2].is_ascii_digit()
        && suffix[3].is_ascii_digit()
        && suffix[4].is_ascii_digit()
        && suffix[5] == b'-'
        && suffix[6].is_ascii_digit()
        && suffix[7].is_ascii_digit()
        && suffix[8] == b'-'
        && suffix[9].is_ascii_digit()
        && suffix[10].is_ascii_digit();
    if !is_dated_suffix {
        return None;
    }
    let base = &model[..model.len() - DATED_SUFFIX_LEN];
    if base.is_empty() { None } else { Some(base) }
}

pub(crate) fn is_gpt_5_4_long_context_surcharge_model(model: &str) -> bool {
    let base = dated_model_alias_base(model).unwrap_or(model);
    matches!(base, "gpt-5.4" | "gpt-5.4-pro")
}

pub(crate) fn proxy_price_version(catalog_version: &str, pricing_mode: ProxyPricingMode) -> String {
    format!("{catalog_version}{}", pricing_mode.price_version_suffix())
}

pub(crate) fn pricing_backfill_attempt_version(catalog: &PricingCatalog) -> String {
    fn mix_fvn1a(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    }

    let mut hash = 0xcbf29ce484222325_u64;
    mix_fvn1a(&mut hash, COST_BACKFILL_ALGO_VERSION.as_bytes());
    mix_fvn1a(&mut hash, &[0xfc]);
    mix_fvn1a(&mut hash, catalog.version.as_bytes());
    mix_fvn1a(&mut hash, &[0xff]);
    mix_fvn1a(&mut hash, API_KEYS_BILLING_ACCOUNT_KIND.as_bytes());
    mix_fvn1a(&mut hash, &[0xfb]);
    mix_fvn1a(&mut hash, REQUESTED_TIER_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xfa]);
    mix_fvn1a(&mut hash, RESPONSE_TIER_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xf9]);
    mix_fvn1a(&mut hash, EXPLICIT_BILLING_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xf8]);

    let mut models = catalog.models.iter().collect::<Vec<_>>();
    models.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (model, pricing) in models {
        mix_fvn1a(&mut hash, model.as_bytes());
        mix_fvn1a(&mut hash, &[0xfe]);
        mix_fvn1a(&mut hash, &pricing.input_per_1m.to_bits().to_le_bytes());
        mix_fvn1a(&mut hash, &pricing.output_per_1m.to_bits().to_le_bytes());

        match pricing.cache_input_per_1m {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        match pricing.reasoning_per_1m {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        mix_fvn1a(&mut hash, &[0xfd]);
    }

    format!("{}@{:016x}", catalog.version, hash)
}

pub(crate) fn estimate_proxy_cost(
    catalog: &PricingCatalog,
    model: Option<&str>,
    usage: &ParsedUsage,
    billing_service_tier: Option<&str>,
    pricing_mode: ProxyPricingMode,
) -> (Option<f64>, bool, Option<String>) {
    let price_version = Some(proxy_price_version(&catalog.version, pricing_mode));
    let Some(model) = model else {
        return (None, false, price_version);
    };
    let Some(pricing) = resolve_pricing_for_model(catalog, model) else {
        return (None, false, price_version);
    };
    let input_tokens = usage.input_tokens.unwrap_or(0).max(0);
    let output_tokens = usage.output_tokens.unwrap_or(0).max(0) as f64;
    let cache_input_tokens = usage.cache_input_tokens.unwrap_or(0).max(0);
    let reasoning_tokens = usage.reasoning_tokens.unwrap_or(0).max(0) as f64;
    if !has_billable_usage(usage) {
        return (None, false, price_version);
    }

    let apply_long_context_surcharge = is_gpt_5_4_long_context_surcharge_model(model)
        && input_tokens > GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS;
    let apply_priority_billing_multiplier = billing_service_tier
        .and_then(normalize_service_tier)
        .as_deref()
        .is_some_and(|tier| tier == PRIORITY_SERVICE_TIER);

    let billable_cache_tokens = if pricing.cache_input_per_1m.is_some() {
        cache_input_tokens
    } else {
        0
    };
    let non_cached_input_tokens = input_tokens.saturating_sub(billable_cache_tokens);

    let non_cached_input_cost =
        (non_cached_input_tokens as f64 / 1_000_000.0) * pricing.input_per_1m;
    let cache_input_cost = pricing
        .cache_input_per_1m
        .map(|cache_price| (billable_cache_tokens as f64 / 1_000_000.0) * cache_price)
        .unwrap_or(0.0);
    let mut input_cost = non_cached_input_cost + cache_input_cost;

    let mut output_cost = (output_tokens / 1_000_000.0) * pricing.output_per_1m;

    let mut reasoning_cost = pricing
        .reasoning_per_1m
        .map(|reasoning_price| (reasoning_tokens / 1_000_000.0) * reasoning_price)
        .unwrap_or(0.0);

    if apply_long_context_surcharge {
        input_cost *= 2.0;
        output_cost *= 1.5;
        reasoning_cost *= 1.5;
    }

    if apply_priority_billing_multiplier {
        input_cost *= 2.0;
        output_cost *= 2.0;
        reasoning_cost *= 2.0;
    }

    let cost = input_cost + output_cost + reasoning_cost;

    (Some(cost), true, price_version)
}

pub(crate) async fn store_raw_payload_file(
    config: &AppConfig,
    invoke_id: &str,
    kind: &str,
    bytes: &[u8],
) -> RawPayloadMeta {
    let mut meta = RawPayloadMeta {
        path: None,
        size_bytes: bytes.len() as i64,
        truncated: false,
        truncated_reason: None,
    };

    if bytes.is_empty() {
        return meta;
    }

    let mut write_len = bytes.len();
    if let Some(limit) = config.proxy_raw_max_bytes
        && write_len > limit
    {
        write_len = limit;
        meta.truncated = true;
        meta.truncated_reason = Some("max_bytes_exceeded".to_string());
    }
    let content = &bytes[..write_len];

    let raw_dir = config.resolved_proxy_raw_dir();

    if let Err(err) = tokio::fs::create_dir_all(&raw_dir).await {
        meta.truncated = true;
        meta.truncated_reason = Some(format!("write_failed:{err}"));
        return meta;
    }

    let filename = format!("{invoke_id}-{kind}.bin");
    let path = raw_dir.join(filename);
    match tokio::fs::write(&path, content).await {
        Ok(_) => {
            meta.path = Some(path.to_string_lossy().to_string());
        }
        Err(err) => {
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{err}"));
        }
    }
    meta
}

pub(crate) async fn broadcast_proxy_capture_follow_up(
    pool: &Pool<Sqlite>,
    hourly_rollup_sync_lock: &Mutex<()>,
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: &Mutex<BroadcastStateCache>,
    relay_config: Option<&CrsStatsConfig>,
    invocation_max_days: u64,
    invoke_id: &str,
) {
    refresh_hourly_rollups_for_read_surfaces_best_effort(
        pool,
        hourly_rollup_sync_lock,
        "proxy capture follow-up",
    )
    .await;

    if broadcaster.receiver_count() == 0 {
        return;
    }

    match collect_summary_snapshots(pool, relay_config, invocation_max_days).await {
        Ok(summaries) => {
            for summary in summaries {
                if let Err(err) = broadcast_summary_if_changed(
                    broadcaster,
                    broadcast_state_cache,
                    &summary.window,
                    summary.summary,
                )
                .await
                {
                    warn!(
                        ?err,
                        invoke_id = %invoke_id,
                        window = %summary.window,
                        "failed to broadcast proxy summary payload"
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                ?err,
                invoke_id = %invoke_id,
                "failed to collect summary snapshots after proxy capture persistence"
            );
        }
    }

    if broadcaster.receiver_count() == 0 {
        return;
    }

    match QuotaSnapshotResponse::fetch_latest(pool).await {
        Ok(Some(snapshot)) => {
            if let Err(err) =
                broadcast_quota_if_changed(broadcaster, broadcast_state_cache, snapshot).await
            {
                warn!(
                    ?err,
                    invoke_id = %invoke_id,
                    "failed to broadcast proxy quota snapshot"
                );
            }
        }
        Ok(None) => {}
        Err(err) => {
            warn!(
                ?err,
                invoke_id = %invoke_id,
                "failed to fetch latest quota snapshot after proxy capture persistence"
            );
        }
    }
}

pub(crate) struct SummaryQuotaBroadcastIdleContext<'a> {
    pub(crate) latest_broadcast_seq: &'a AtomicU64,
    pub(crate) broadcast_running: &'a AtomicBool,
    pub(crate) shutdown: &'a CancellationToken,
    pub(crate) pool: &'a Pool<Sqlite>,
    pub(crate) hourly_rollup_sync_lock: &'a Mutex<()>,
    pub(crate) broadcaster: &'a broadcast::Sender<BroadcastPayload>,
    pub(crate) broadcast_state_cache: &'a Mutex<BroadcastStateCache>,
    pub(crate) relay_config: Option<&'a CrsStatsConfig>,
    pub(crate) invocation_max_days: u64,
    pub(crate) invoke_id: &'a str,
}

pub(crate) async fn finish_summary_quota_broadcast_idle(
    ctx: SummaryQuotaBroadcastIdleContext<'_>,
    synced_seq: u64,
) -> bool {
    ctx.broadcast_running.store(false, Ordering::Release);

    let pending_seq = ctx.latest_broadcast_seq.load(Ordering::Acquire);
    if pending_seq == synced_seq {
        return false;
    }

    if ctx.shutdown.is_cancelled() {
        info!(
            invoke_id = %ctx.invoke_id,
            pending_seq,
            synced_seq,
            "flushing final summary/quota snapshots inline because shutdown arrived during broadcast worker idle handoff"
        );
        broadcast_proxy_capture_follow_up(
            ctx.pool,
            ctx.hourly_rollup_sync_lock,
            ctx.broadcaster,
            ctx.broadcast_state_cache,
            ctx.relay_config,
            ctx.invocation_max_days,
            ctx.invoke_id,
        )
        .await;
        return false;
    }

    ctx.broadcast_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub(crate) async fn schedule_proxy_capture_follow_up_worker(
    state: &AppState,
    invoke_id: &str,
) -> Result<()> {
    if state.shutdown.is_cancelled() {
        info!(
            invoke_id = %invoke_id,
            "broadcasting final summary/quota snapshots inline because shutdown is in progress"
        );
        broadcast_proxy_capture_follow_up(
            &state.pool,
            state.hourly_rollup_sync_lock.as_ref(),
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            state.config.crs_stats.as_ref(),
            state.config.invocation_max_days,
            invoke_id,
        )
        .await;
        return Ok(());
    }

    state
        .proxy_summary_quota_broadcast_seq
        .fetch_add(1, Ordering::Relaxed);
    if state.shutdown.is_cancelled() {
        info!(
            invoke_id = %invoke_id,
            "broadcasting final summary/quota snapshots inline because shutdown started after record broadcast"
        );
        broadcast_proxy_capture_follow_up(
            &state.pool,
            state.hourly_rollup_sync_lock.as_ref(),
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            state.config.crs_stats.as_ref(),
            state.config.invocation_max_days,
            invoke_id,
        )
        .await;
        return Ok(());
    }
    if state
        .proxy_summary_quota_broadcast_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(());
    }

    let latest_broadcast_seq = state.proxy_summary_quota_broadcast_seq.clone();
    let broadcast_running = state.proxy_summary_quota_broadcast_running.clone();
    let pool = state.pool.clone();
    let hourly_rollup_sync_lock = state.hourly_rollup_sync_lock.clone();
    let broadcaster = state.broadcaster.clone();
    let broadcast_state_cache = state.broadcast_state_cache.clone();
    let relay_config = state.config.crs_stats.clone();
    let invocation_max_days = state.config.invocation_max_days;
    let shutdown = state.shutdown.clone();
    let broadcast_handle_slot = state.proxy_summary_quota_broadcast_handle.clone();
    let invoke_id = invoke_id.to_string();
    let handle = tokio::spawn(async move {
        let mut synced_seq = 0_u64;
        loop {
            let target_seq = latest_broadcast_seq.load(Ordering::Acquire);
            if shutdown.is_cancelled() {
                if target_seq != synced_seq {
                    info!(
                        invoke_id = %invoke_id,
                        "flushing final summary/quota snapshots inline before shutdown"
                    );
                    broadcast_proxy_capture_follow_up(
                        &pool,
                        hourly_rollup_sync_lock.as_ref(),
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        relay_config.as_ref(),
                        invocation_max_days,
                        &invoke_id,
                    )
                    .await;
                }
                broadcast_running.store(false, Ordering::Release);
                info!(
                    invoke_id = %invoke_id,
                    "stopping summary/quota broadcast worker because shutdown is in progress"
                );
                break;
            }

            if target_seq == synced_seq {
                if finish_summary_quota_broadcast_idle(
                    SummaryQuotaBroadcastIdleContext {
                        latest_broadcast_seq: latest_broadcast_seq.as_ref(),
                        broadcast_running: broadcast_running.as_ref(),
                        shutdown: &shutdown,
                        pool: &pool,
                        hourly_rollup_sync_lock: hourly_rollup_sync_lock.as_ref(),
                        broadcaster: &broadcaster,
                        broadcast_state_cache: broadcast_state_cache.as_ref(),
                        relay_config: relay_config.as_ref(),
                        invocation_max_days,
                        invoke_id: &invoke_id,
                    },
                    synced_seq,
                )
                .await
                {
                    continue;
                }
                break;
            }
            synced_seq = target_seq;

            tokio::select! {
                _ = shutdown.cancelled() => {
                    broadcast_proxy_capture_follow_up(
                        &pool,
                        hourly_rollup_sync_lock.as_ref(),
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        relay_config.as_ref(),
                        invocation_max_days,
                        &invoke_id,
                    )
                    .await;
                    broadcast_running.store(false, Ordering::Release);
                    info!(
                        invoke_id = %invoke_id,
                        "summary/quota broadcast worker flushed follow-up before shutdown"
                    );
                    break;
                }
                _ = broadcast_proxy_capture_follow_up(
                    &pool,
                    hourly_rollup_sync_lock.as_ref(),
                    &broadcaster,
                    broadcast_state_cache.as_ref(),
                    relay_config.as_ref(),
                    invocation_max_days,
                    &invoke_id,
                ) => {}
            };
        }
    });

    let finished_handles = {
        let mut guard = broadcast_handle_slot.lock().await;
        let mut active_handles = std::mem::take(&mut *guard);
        let mut finished_handles = Vec::new();
        let mut idx = 0;
        while idx < active_handles.len() {
            if active_handles[idx].is_finished() {
                finished_handles.push(active_handles.remove(idx));
            } else {
                idx += 1;
            }
        }
        active_handles.push(handle);
        *guard = active_handles;
        finished_handles
    };
    for finished_handle in finished_handles {
        if let Err(err) = finished_handle.await {
            error!(
                ?err,
                "summary/quota broadcast worker terminated unexpectedly"
            );
        }
    }

    Ok(())
}

pub(crate) async fn persist_and_broadcast_proxy_capture(
    state: &AppState,
    capture_started: Instant,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let inserted = persist_proxy_capture_record(&state.pool, capture_started, record).await?;
    let Some(inserted_record) = inserted else {
        return Ok(());
    };
    if inserted_record
        .prompt_cache_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }

    let invoke_id = inserted_record.invoke_id.clone();
    if state.broadcaster.receiver_count() > 0
        && let Err(err) = state.broadcaster.send(BroadcastPayload::Records {
            records: vec![inserted_record],
        })
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast new proxy capture record"
        );
    }
    schedule_proxy_capture_follow_up_worker(state, &invoke_id).await
}

pub(crate) async fn persist_proxy_capture_record(
    pool: &Pool<Sqlite>,
    capture_started: Instant,
    mut record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
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

    let mut tx = pool.begin().await?;
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
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36
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
    .bind(record.cost_estimated as i64)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure_kind.as_deref())
    .bind(failure.failure_class.as_str())
    .bind(failure.is_actionable as i64)
    .bind(record.payload.as_deref())
    .bind(&record.raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
    .bind(record.req_raw.truncated as i64)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(record.resp_raw.path.as_deref())
    .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
    .bind(record.resp_raw.truncated as i64)
    .bind(record.resp_raw.truncated_reason.as_deref())
    .bind(None::<f64>)
    .bind(record.timings.t_req_read_ms)
    .bind(record.timings.t_req_parse_ms)
    .bind(record.timings.t_upstream_connect_ms)
    .bind(record.timings.t_upstream_ttfb_ms)
    .bind(record.timings.t_upstream_stream_ms)
    .bind(record.timings.t_resp_parse_ms)
    .bind(None::<f64>)
    .bind(created_at)
    .execute(tx.as_mut())
    .await?;

    let (invocation_id, inserted_new_row) = if insert_result.rows_affected() > 0 {
        (insert_result.last_insert_rowid(), true)
    } else {
        let Some(existing) = load_persisted_invocation_identity_tx(
            tx.as_mut(),
            &record.invoke_id,
            &record.occurred_at,
        )
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let allow_terminal_repair = !invocation_status_is_in_flight(Some(record.status.as_str()))
            && invocation_status_is_recoverable_proxy_interrupted(
                existing.status.as_deref(),
                existing.failure_kind.as_deref(),
            );
        if !invocation_status_is_in_flight(existing.status.as_deref()) && !allow_terminal_repair {
            tx.commit().await?;
            return Ok(None);
        }

        let affected = sqlx::query(
            r#"
            UPDATE codex_invocations
            SET source = ?2,
                model = ?3,
                input_tokens = ?4,
                output_tokens = ?5,
                cache_input_tokens = ?6,
                reasoning_tokens = ?7,
                total_tokens = ?8,
                cost = ?9,
                cost_estimated = ?10,
                price_version = ?11,
                status = ?12,
                error_message = ?13,
                failure_kind = ?14,
                failure_class = ?15,
                is_actionable = ?16,
                payload = ?17,
                raw_response = ?18,
                request_raw_path = ?19,
                request_raw_size = ?20,
                request_raw_truncated = ?21,
                request_raw_truncated_reason = ?22,
                response_raw_path = ?23,
                response_raw_size = ?24,
                response_raw_truncated = ?25,
                response_raw_truncated_reason = ?26,
                t_total_ms = ?27,
                t_req_read_ms = ?28,
                t_req_parse_ms = ?29,
                t_upstream_connect_ms = ?30,
                t_upstream_ttfb_ms = ?31,
                t_upstream_stream_ms = ?32,
                t_resp_parse_ms = ?33,
                t_persist_ms = ?34
            WHERE id = ?1
              AND (
                    LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                    OR (
                        LOWER(TRIM(COALESCE(status, ''))) = 'interrupted'
                        AND LOWER(TRIM(COALESCE(failure_kind, ''))) = 'proxy_interrupted'
                    )
              )
            "#,
        )
        .bind(existing.id)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(&record.raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(record.resp_raw.path.as_deref())
        .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
        .bind(record.resp_raw.truncated as i64)
        .bind(record.resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(record.timings.t_req_read_ms)
        .bind(record.timings.t_req_parse_ms)
        .bind(record.timings.t_upstream_connect_ms)
        .bind(record.timings.t_upstream_ttfb_ms)
        .bind(record.timings.t_upstream_stream_ms)
        .bind(record.timings.t_resp_parse_ms)
        .bind(None::<f64>)
        .execute(tx.as_mut())
        .await?
        .rows_affected();
        if affected == 0 {
            tx.commit().await?;
            return Ok(None);
        }

        (existing.id, false)
    };

    touch_invocation_upstream_account_last_activity_tx(
        tx.as_mut(),
        &record.occurred_at,
        record.payload.as_deref(),
    )
    .await?;

    if inserted_new_row {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &[InvocationHourlySourceRecord {
                id: invocation_id,
                occurred_at: record.occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some(record.status.clone()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                total_tokens: record.usage.total_tokens,
                cost: record.cost,
                error_message: record.error_message.clone(),
                failure_kind: failure_kind.clone(),
                failure_class: Some(failure.failure_class.as_str().to_string()),
                is_actionable: Some(failure.is_actionable as i64),
                payload: record.payload.clone(),
                t_total_ms: None,
                t_req_read_ms: Some(record.timings.t_req_read_ms),
                t_req_parse_ms: Some(record.timings.t_req_parse_ms),
                t_upstream_connect_ms: Some(record.timings.t_upstream_connect_ms),
                t_upstream_ttfb_ms: Some(record.timings.t_upstream_ttfb_ms),
                t_upstream_stream_ms: Some(record.timings.t_upstream_stream_ms),
                t_resp_parse_ms: Some(record.timings.t_resp_parse_ms),
                t_persist_ms: None,
            }],
            &INVOCATION_HOURLY_ROLLUP_TARGETS,
        )
        .await?;
    } else {
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[invocation_id]).await?;
    }

    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        invocation_id,
    )
    .await?;

    record.timings.t_persist_ms = elapsed_ms(persist_started);
    record.timings.t_total_ms = elapsed_ms(capture_started);

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET t_total_ms = ?2,
            t_persist_ms = ?3
        WHERE id = ?1
        "#,
    )
    .bind(invocation_id)
    .bind(record.timings.t_total_ms)
    .bind(record.timings.t_persist_ms)
    .execute(tx.as_mut())
    .await?;

    recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[invocation_id]).await?;

    let persisted =
        load_persisted_api_invocation_tx(tx.as_mut(), &record.invoke_id, &record.occurred_at)
            .await?;
    tx.commit().await?;

    Ok(Some(persisted))
}

pub(crate) fn read_proxy_raw_bytes(path: &str, fallback_root: Option<&Path>) -> io::Result<Vec<u8>> {
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
    } else {
        Ok(bytes)
    }
}

pub(crate) async fn current_proxy_usage_backfill_snapshot_max_id(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar(
        r#"
        SELECT COALESCE(MAX(id), 0)
        FROM codex_invocations
        WHERE source = ?1
          AND status = 'success'
          AND total_tokens IS NULL
          AND response_raw_path IS NOT NULL
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await?)
}

pub(crate) async fn backfill_proxy_usage_tokens_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyUsageBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyUsageBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyUsageBackfillCandidate>(
            r#"
            SELECT id, response_raw_path, payload
            FROM codex_invocations
            WHERE source = ?1
              AND status = 'success'
              AND total_tokens IS NULL
              AND response_raw_path IS NOT NULL
              AND id > ?2
              AND id <= ?3
            ORDER BY id ASC
            LIMIT ?4
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(snapshot_max_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut updates = Vec::new();
        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_response =
                match read_proxy_raw_bytes(&candidate.response_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} response_raw_path={} reason=missing_file",
                                candidate.id, candidate.response_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let (target, is_stream) = parse_proxy_capture_summary(candidate.payload.as_deref());
            let (payload_for_parse, decode_error) =
                decode_response_payload_for_usage(&raw_response, None);
            let response_info =
                parse_target_response_payload(target, payload_for_parse.as_ref(), is_stream, None);
            let usage = response_info.usage;
            let has_usage = usage.total_tokens.is_some()
                || usage.input_tokens.is_some()
                || usage.output_tokens.is_some()
                || usage.cache_input_tokens.is_some()
                || usage.reasoning_tokens.is_some();
            if !has_usage {
                if decode_error.is_some() {
                    summary.skipped_decode_error += 1;
                } else {
                    summary.skipped_without_usage += 1;
                }
                continue;
            }

            updates.push(ProxyUsageBackfillUpdate {
                id: candidate.id,
                usage,
            });
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_this_batch = 0_u64;
            let mut updated_ids = Vec::new();
            for update in updates {
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET input_tokens = ?1,
                        output_tokens = ?2,
                        cache_input_tokens = ?3,
                        reasoning_tokens = ?4,
                        total_tokens = ?5
                    WHERE id = ?6
                      AND source = ?7
                      AND total_tokens IS NULL
                    "#,
                )
                .bind(update.usage.input_tokens)
                .bind(update.usage.output_tokens)
                .bind(update.usage.cache_input_tokens)
                .bind(update.usage.reasoning_tokens)
                .bind(update.usage.total_tokens)
                .bind(update.id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                updated_this_batch += affected;
                if affected > 0 {
                    updated_ids.push(update.id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
            }
            tx.commit().await?;
            summary.updated += updated_this_batch;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
pub(crate) async fn backfill_proxy_usage_tokens(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(pool).await?;
    Ok(backfill_proxy_usage_tokens_from_cursor(
        pool,
        0,
        snapshot_max_id,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
pub(crate) async fn backfill_proxy_usage_tokens_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    Ok(backfill_proxy_usage_tokens_from_cursor(
        pool,
        0,
        snapshot_max_id,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
pub(crate) async fn run_backfill_with_retry(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let mut attempt = 1_u32;
    loop {
        match backfill_proxy_usage_tokens(pool, raw_path_fallback_root).await {
            Ok(summary) => return Ok(summary),
            Err(err)
                if attempt < BACKFILL_LOCK_RETRY_MAX_ATTEMPTS && is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = BACKFILL_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_secs = BACKFILL_LOCK_RETRY_DELAY_SECS,
                    error = %err,
                    "proxy usage startup backfill hit sqlite lock; retrying"
                );
                attempt += 1;
                sleep(Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS)).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "proxy usage startup backfill failed after {attempt}/{} attempt(s)",
                        BACKFILL_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

pub(crate) async fn current_proxy_cost_backfill_snapshot_max_id(
    pool: &Pool<Sqlite>,
    attempt_version: &str,
    requested_tier_price_version: &str,
    response_tier_price_version: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        r#"
        WITH base AS (
            SELECT
                inv.id,
                inv.cost,
                inv.price_version,
                CASE
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requestedServiceTier') = 'text'
                    THEN json_extract(inv.payload, '$.requestedServiceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requested_service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.requested_service_tier')
                END AS requested_service_tier,
                CASE
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billingServiceTier') = 'text'
                    THEN json_extract(inv.payload, '$.billingServiceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billing_service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.billing_service_tier')
                END AS billing_service_tier,
                CASE
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.serviceTier') = 'text'
                    THEN json_extract(inv.payload, '$.serviceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.service_tier')
                END AS service_tier,
                CASE
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamAccountKind') = 'text'
                    THEN json_extract(inv.payload, '$.upstreamAccountKind')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_account_kind') = 'text'
                    THEN json_extract(inv.payload, '$.upstream_account_kind')
                END AS snapshot_upstream_account_kind,
                acc.kind AS live_upstream_account_kind,
                CASE
                  WHEN acc.created_at IS NOT NULL
                    AND TRIM(CAST(acc.created_at AS TEXT)) != ''
                    AND inv.occurred_at IS NOT NULL
                    AND TRIM(CAST(inv.occurred_at AS TEXT)) != ''
                    AND julianday(acc.created_at) <= julianday(inv.occurred_at)
                    AND (
                        acc.updated_at IS NULL
                        OR TRIM(CAST(acc.updated_at AS TEXT)) = ''
                        OR julianday(acc.updated_at) <= julianday(inv.occurred_at)
                    )
                  THEN 1
                  ELSE 0
                END AS live_upstream_account_snapshot_safe
            FROM codex_invocations inv
            LEFT JOIN pool_upstream_accounts acc
              ON acc.id = CASE
                  WHEN json_valid(inv.payload)
                    THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER)
                END
            WHERE inv.source = ?1
              AND LOWER(TRIM(COALESCE(inv.status, ''))) IN ('success', 'failed')
              AND inv.model IS NOT NULL
              AND (
                  COALESCE(inv.input_tokens, 0) > 0
                  OR COALESCE(inv.output_tokens, 0) > 0
                  OR COALESCE(inv.cache_input_tokens, 0) > 0
                  OR COALESCE(inv.reasoning_tokens, 0) > 0
              )
        ),
        cost_candidates AS (
            SELECT
                *,
                CASE
                  WHEN LOWER(TRIM(COALESCE(
                        snapshot_upstream_account_kind,
                        CASE WHEN live_upstream_account_snapshot_safe = 1 THEN live_upstream_account_kind END,
                        ''
                    ))) = ?4
                    AND TRIM(COALESCE(requested_service_tier, '')) != ''
                  THEN 1
                  ELSE 0
                END AS uses_requested_tier_strategy
            FROM base
        )
        SELECT COALESCE(MAX(id), 0)
        FROM cost_candidates
        WHERE (
            uses_requested_tier_strategy = 1
            AND (
                LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(requested_service_tier, '')))
                OR (cost IS NULL AND (price_version IS NULL OR price_version != ?2))
                OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?3))
            )
        )
        OR (
            uses_requested_tier_strategy = 0
            AND (
                LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(service_tier, '')))
                OR (cost IS NULL AND (price_version IS NULL OR price_version != ?2))
                OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?5))
            )
        )
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(attempt_version)
    .bind(requested_tier_price_version)
    .bind(API_KEYS_BILLING_ACCOUNT_KIND)
    .bind(response_tier_price_version)
    .fetch_one(pool)
    .await?)
}

pub(crate) async fn backfill_proxy_missing_costs_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    snapshot_max_id: i64,
    catalog: &PricingCatalog,
    attempt_version: &str,
    requested_tier_price_version: &str,
    response_tier_price_version: &str,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyCostBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyCostBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyCostBackfillCandidate>(
            r#"
            WITH base AS (
                SELECT
                    inv.id,
                    inv.model,
                    inv.input_tokens,
                    inv.output_tokens,
                    inv.cache_input_tokens,
                    inv.reasoning_tokens,
                    inv.total_tokens,
                    inv.cost,
                    inv.price_version,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requestedServiceTier') = 'text'
                        THEN json_extract(inv.payload, '$.requestedServiceTier')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requested_service_tier') = 'text'
                        THEN json_extract(inv.payload, '$.requested_service_tier')
                    END AS requested_service_tier,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.serviceTier') = 'text'
                        THEN json_extract(inv.payload, '$.serviceTier')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.service_tier') = 'text'
                        THEN json_extract(inv.payload, '$.service_tier')
                    END AS service_tier,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billingServiceTier') = 'text'
                        THEN json_extract(inv.payload, '$.billingServiceTier')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billing_service_tier') = 'text'
                        THEN json_extract(inv.payload, '$.billing_service_tier')
                    END AS billing_service_tier,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamAccountKind') = 'text'
                        THEN json_extract(inv.payload, '$.upstreamAccountKind')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_account_kind') = 'text'
                        THEN json_extract(inv.payload, '$.upstream_account_kind')
                    END AS snapshot_upstream_account_kind,
                    CASE
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamBaseUrlHost') = 'text'
                        THEN json_extract(inv.payload, '$.upstreamBaseUrlHost')
                      WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_base_url_host') = 'text'
                        THEN json_extract(inv.payload, '$.upstream_base_url_host')
                    END AS snapshot_upstream_base_url_host,
                    acc.kind AS live_upstream_account_kind,
                    CASE
                      WHEN acc.upstream_base_url IS NULL OR TRIM(CAST(acc.upstream_base_url AS TEXT)) = '' THEN NULL
                      ELSE
                        CASE
                          WHEN INSTR(
                            CASE
                              WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                                THEN SUBSTR(
                                  REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''),
                                  1,
                                  INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1
                                )
                              ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/')
                            END,
                            ':'
                          ) > 0
                            THEN SUBSTR(
                              CASE
                                WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                                  THEN SUBSTR(
                                    REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''),
                                    1,
                                    INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1
                                  )
                                ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/')
                              END,
                              1,
                              INSTR(
                                CASE
                                  WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                                    THEN SUBSTR(
                                      REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''),
                                      1,
                                      INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1
                                    )
                                  ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/')
                                END,
                                ':'
                              ) - 1
                            )
                          ELSE
                            CASE
                              WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                                THEN SUBSTR(
                                  REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''),
                                  1,
                                  INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1
                                )
                              ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/')
                            END
                        END
                    END AS live_upstream_base_url_host,
                    CASE
                      WHEN acc.created_at IS NOT NULL
                        AND TRIM(CAST(acc.created_at AS TEXT)) != ''
                        AND inv.occurred_at IS NOT NULL
                        AND TRIM(CAST(inv.occurred_at AS TEXT)) != ''
                        AND julianday(acc.created_at) <= julianday(inv.occurred_at)
                        AND (
                            acc.updated_at IS NULL
                            OR TRIM(CAST(acc.updated_at AS TEXT)) = ''
                            OR julianday(acc.updated_at) <= julianday(inv.occurred_at)
                        )
                      THEN 1
                      ELSE 0
                    END AS live_upstream_account_snapshot_safe
                FROM codex_invocations inv
                LEFT JOIN pool_upstream_accounts acc
                  ON acc.id = CASE
                      WHEN json_valid(inv.payload)
                        THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER)
                    END
                WHERE inv.source = ?1
                  AND LOWER(TRIM(COALESCE(inv.status, ''))) IN ('success', 'failed')
                  AND inv.model IS NOT NULL
                  AND (
                      COALESCE(inv.input_tokens, 0) > 0
                      OR COALESCE(inv.output_tokens, 0) > 0
                      OR COALESCE(inv.cache_input_tokens, 0) > 0
                      OR COALESCE(inv.reasoning_tokens, 0) > 0
                  )
                  AND inv.id > ?2
                  AND inv.id <= ?3
            ),
            cost_candidates AS (
                SELECT
                    *,
                    CASE
                      WHEN LOWER(TRIM(COALESCE(
                            snapshot_upstream_account_kind,
                            CASE WHEN live_upstream_account_snapshot_safe = 1 THEN live_upstream_account_kind END,
                            ''
                        ))) = ?6
                        AND TRIM(COALESCE(requested_service_tier, '')) != ''
                      THEN 1
                      ELSE 0
                    END AS uses_requested_tier_strategy
                FROM base
            )
            SELECT
                id,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
                total_tokens,
                requested_service_tier,
                service_tier,
                snapshot_upstream_account_kind,
                snapshot_upstream_base_url_host,
                live_upstream_base_url_host,
                live_upstream_account_kind,
                live_upstream_account_snapshot_safe
            FROM cost_candidates
            WHERE (
                uses_requested_tier_strategy = 1
                AND (
                    LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(requested_service_tier, '')))
                    OR (cost IS NULL AND (price_version IS NULL OR price_version != ?4))
                    OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?5))
                )
            )
            OR (
                uses_requested_tier_strategy = 0
                AND (
                    LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(service_tier, '')))
                    OR (cost IS NULL AND (price_version IS NULL OR price_version != ?4))
                    OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?7))
                )
            )
            ORDER BY id ASC
            LIMIT ?8
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(snapshot_max_id)
        .bind(attempt_version)
        .bind(requested_tier_price_version)
        .bind(API_KEYS_BILLING_ACCOUNT_KIND)
        .bind(response_tier_price_version)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut updates = Vec::new();
        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;
            let Some(model) = candidate.model.as_deref() else {
                summary.skipped_unpriced_model += 1;
                continue;
            };
            let usage = ParsedUsage {
                input_tokens: candidate.input_tokens,
                output_tokens: candidate.output_tokens,
                cache_input_tokens: candidate.cache_input_tokens,
                reasoning_tokens: candidate.reasoning_tokens,
                total_tokens: candidate.total_tokens,
            };
            if !has_billable_usage(&usage) {
                summary.skipped_unpriced_model += 1;
                continue;
            }

            let allow_live_fallback =
                allow_live_upstream_account_fallback(Some(
                    candidate.live_upstream_account_snapshot_safe,
                ));
            let upstream_account_kind = resolve_backfill_upstream_account_kind(
                candidate.snapshot_upstream_account_kind.as_deref(),
                candidate.live_upstream_account_kind.as_deref(),
                allow_live_fallback,
            );
            let upstream_base_url_host = resolve_backfill_upstream_base_url_host(
                candidate.snapshot_upstream_base_url_host.as_deref(),
                candidate.live_upstream_base_url_host.as_deref(),
                allow_live_fallback,
            );
            let (billing_service_tier, pricing_mode) =
                resolve_proxy_billing_service_tier_and_pricing_mode(
                    None,
                    candidate.requested_service_tier.as_deref(),
                    candidate.service_tier.as_deref(),
                    upstream_account_kind.as_deref(),
                );
            let (cost, cost_estimated, price_version) = estimate_proxy_cost(
                catalog,
                Some(model),
                &usage,
                billing_service_tier.as_deref(),
                pricing_mode,
            );
            if cost.is_none() || !cost_estimated {
                summary.skipped_unpriced_model += 1;
                push_backfill_sample(
                    &mut samples,
                    format!("id={} model={} reason=unpriced_model", candidate.id, model),
                );
            }
            let persisted_price_version = if cost_estimated && cost.is_some() {
                price_version
            } else {
                Some(attempt_version.to_string())
            };
            updates.push(ProxyCostBackfillUpdate {
                id: candidate.id,
                cost,
                cost_estimated,
                price_version: persisted_price_version,
                billing_service_tier,
                upstream_account_kind,
                upstream_base_url_host,
            });
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_this_batch = 0_u64;
            let mut updated_ids = Vec::new();
            for update in updates {
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET payload = json_set(
                            json_set(
                                json_set(
                                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                                    '$.billingServiceTier',
                                    ?1
                                ),
                                '$.upstreamAccountKind',
                                ?2
                            ),
                            '$.upstreamBaseUrlHost',
                            ?3
                        ),
                        cost = ?4,
                        cost_estimated = ?5,
                        price_version = ?6
                    WHERE id = ?7
                      AND source = ?8
                    "#,
                )
                .bind(update.billing_service_tier.as_deref())
                .bind(update.upstream_account_kind.as_deref())
                .bind(update.upstream_base_url_host.as_deref())
                .bind(update.cost)
                .bind(update.cost_estimated as i64)
                .bind(update.price_version.as_deref())
                .bind(update.id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                updated_this_batch += affected;
                if affected > 0 {
                    updated_ids.push(update.id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
            }
            tx.commit().await?;
            summary.updated += updated_this_batch;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}
