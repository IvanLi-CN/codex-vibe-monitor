use super::*;
pub(crate) async fn store_raw_payload_file(
    config: &AppConfig,
    invoke_id: &str,
    kind: &str,
    bytes: Bytes,
) -> RawPayloadMeta {
    let started = Instant::now();
    let mut meta = RawPayloadMeta {
        path: None,
        size_bytes: bytes.len() as i64,
        truncated: false,
        truncated_reason: None,
    };

    if bytes.is_empty() {
        return meta;
    }

    let mut content = bytes;
    if let Some(limit) = config.proxy_raw_max_bytes
        && content.len() > limit
    {
        content = content.slice(..limit);
        meta.truncated = true;
        meta.truncated_reason = Some("max_bytes_exceeded".to_string());
    }

    let raw_dir = config.resolved_proxy_raw_dir();
    let file_bytes = content.len();
    let codec = raw_payload_codec(config, file_bytes);
    let path = raw_payload_path(&raw_dir, invoke_id, kind, codec);
    let write_result = write_raw_payload_content(&raw_dir, &path, codec, content).await;
    match write_result {
        Ok(_) => {
            meta.path = Some(path.to_string_lossy().to_string());
        }
        Err(err) => {
            if codec != RAW_CODEC_IDENTITY {
                let _ = fs::remove_file(&path);
            }
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{err}"));
        }
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        warn!(
            invoke_id,
            raw_kind = kind,
            codec,
            file_bytes,
            observed_bytes = meta.size_bytes,
            truncated = meta.truncated,
            has_path = meta.path.is_some(),
            elapsed_ms,
            "proxy raw payload file write was slow"
        );
    } else {
        debug!(
            invoke_id,
            raw_kind = kind,
            codec,
            file_bytes,
            observed_bytes = meta.size_bytes,
            truncated = meta.truncated,
            has_path = meta.path.is_some(),
            elapsed_ms,
            "proxy raw payload file write completed"
        );
    }
    meta
}

fn raw_payload_codec(config: &AppConfig, file_bytes: usize) -> &'static str {
    let compressed = match config.proxy_raw_compression {
        RawCompressionCodec::Zstd => true,
        RawCompressionCodec::Gzip => config
            .proxy_raw_immediate_compression_threshold()
            .is_some_and(|threshold| file_bytes >= threshold),
        RawCompressionCodec::None => false,
    };
    match (compressed, config.proxy_raw_compression) {
        (true, RawCompressionCodec::Gzip) => RAW_CODEC_GZIP,
        (true, RawCompressionCodec::Zstd) => RAW_CODEC_ZSTD,
        _ => RAW_CODEC_IDENTITY,
    }
}

fn raw_payload_path(raw_dir: &Path, invoke_id: &str, kind: &str, codec: &str) -> PathBuf {
    let plain_path = raw_payload_path_for_kind(raw_dir, invoke_id, kind, false);
    match codec {
        RAW_CODEC_ZSTD => raw_payload_zstd_path(&plain_path),
        RAW_CODEC_GZIP => raw_payload_gzip_path(&plain_path),
        _ => plain_path,
    }
}

async fn write_raw_payload_content(
    raw_dir: &Path,
    path: &Path,
    codec: &'static str,
    content: Bytes,
) -> io::Result<()> {
    if codec == RAW_CODEC_GZIP {
        let write_path = path.to_owned();
        return run_blocking_raw_writer_io(move || {
            let mut encoder = create_gzip_streaming_raw_encoder(&write_path)?;
            encoder.write_all(content.as_ref())?;
            encoder.finish()?.flush()
        })
        .await;
    }
    if codec == RAW_CODEC_ZSTD {
        let write_path = path.to_owned();
        return run_blocking_raw_writer_io(move || {
            let mut encoder = create_zstd_streaming_raw_encoder(&write_path)?;
            encoder.write_all(content.as_ref())?;
            encoder.finish()?.flush()
        })
        .await;
    }
    tokio::fs::create_dir_all(raw_dir).await?;
    tokio::fs::write(path, content).await
}

pub(crate) async fn store_raw_payload_snapshot_file(
    config: &AppConfig,
    invoke_id: &str,
    kind: &str,
    source_path: PathBuf,
    source_size: usize,
) -> RawPayloadMeta {
    let started = Instant::now();
    let file_bytes = config
        .proxy_raw_max_bytes
        .map_or(source_size, |limit| source_size.min(limit));
    let mut meta = RawPayloadMeta {
        path: None,
        size_bytes: source_size as i64,
        truncated: file_bytes < source_size,
        truncated_reason: (file_bytes < source_size).then(|| "max_bytes_exceeded".to_string()),
    };
    let raw_dir = config.resolved_proxy_raw_dir();
    let born_compressed = match config.proxy_raw_compression {
        RawCompressionCodec::Zstd => true,
        RawCompressionCodec::Gzip => config
            .proxy_raw_immediate_compression_threshold()
            .is_some_and(|threshold| file_bytes >= threshold),
        RawCompressionCodec::None => false,
    };
    let codec = match (born_compressed, config.proxy_raw_compression) {
        (true, RawCompressionCodec::Gzip) => RAW_CODEC_GZIP,
        (true, RawCompressionCodec::Zstd) => RAW_CODEC_ZSTD,
        _ => RAW_CODEC_IDENTITY,
    };
    let plain_path = raw_payload_path_for_kind(&raw_dir, invoke_id, kind, false);
    let path = if codec == RAW_CODEC_ZSTD {
        raw_payload_zstd_path(&plain_path)
    } else if codec == RAW_CODEC_GZIP {
        raw_payload_gzip_path(&plain_path)
    } else {
        plain_path
    };
    let write_path = path.clone();
    let write_result = run_blocking_raw_writer_io(move || {
        let mut source = std::io::BufReader::with_capacity(
            REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES,
            fs::File::open(source_path)?,
        )
        .take(file_bytes as u64);
        if codec == RAW_CODEC_GZIP {
            let mut encoder = create_gzip_streaming_raw_encoder(&write_path)?;
            std::io::copy(&mut source, &mut encoder)?;
            encoder.finish()?.flush()
        } else if codec == RAW_CODEC_ZSTD {
            let mut encoder = create_zstd_streaming_raw_encoder(&write_path)?;
            std::io::copy(&mut source, &mut encoder)?;
            encoder.finish()?.flush()
        } else {
            prepare_streaming_raw_parent(&write_path)?;
            let mut writer = fs::File::create(&write_path)?;
            std::io::copy(&mut source, &mut writer)?;
            writer.flush()
        }
    })
    .await;
    match write_result {
        Ok(()) => meta.path = Some(path.to_string_lossy().to_string()),
        Err(err) => {
            let _ = fs::remove_file(&path);
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{err}"));
        }
    }
    debug!(
        invoke_id,
        raw_kind = kind,
        codec,
        file_bytes,
        observed_bytes = meta.size_bytes,
        truncated = meta.truncated,
        has_path = meta.path.is_some(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "proxy raw replay snapshot write completed"
    );
    meta
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProxyCaptureFollowUpBroadcastMode {
    ActiveSubscribers,
    ShutdownFlush,
}

pub(crate) async fn broadcast_proxy_capture_follow_up(
    pool: &Pool<Sqlite>,
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: &Mutex<BroadcastStateCache>,
    subscription_hub: &SubscriptionHub,
    mode: ProxyCaptureFollowUpBroadcastMode,
    invoke_id: &str,
) {
    if matches!(mode, ProxyCaptureFollowUpBroadcastMode::ActiveSubscribers)
        && !subscription_hub
            .has_active_topic_name("quota.current")
            .await
    {
        subscription_hub
            .mark_topic_name_dirty("quota.current")
            .await;
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
    pub(crate) broadcaster: &'a broadcast::Sender<BroadcastPayload>,
    pub(crate) broadcast_state_cache: &'a Mutex<BroadcastStateCache>,
    pub(crate) subscription_hub: &'a SubscriptionHub,
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
            ctx.broadcaster,
            ctx.broadcast_state_cache,
            ctx.subscription_hub,
            ProxyCaptureFollowUpBroadcastMode::ShutdownFlush,
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
            "broadcasting final quota snapshot inline because shutdown is in progress"
        );
        broadcast_proxy_capture_follow_up(
            &state.pool,
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            state.subscription_hub.as_ref(),
            ProxyCaptureFollowUpBroadcastMode::ShutdownFlush,
            invoke_id,
        )
        .await;
        return Ok(());
    }

    if !state
        .subscription_hub
        .has_active_topic_name("quota.current")
        .await
    {
        state
            .subscription_hub
            .mark_topic_name_dirty("quota.current")
            .await;
        return Ok(());
    }

    state
        .proxy_summary_quota_broadcast_seq
        .fetch_add(1, Ordering::Relaxed);
    if state.shutdown.is_cancelled() {
        info!(
            invoke_id = %invoke_id,
            "broadcasting final quota snapshot inline because shutdown started after record broadcast"
        );
        broadcast_proxy_capture_follow_up(
            &state.pool,
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            state.subscription_hub.as_ref(),
            ProxyCaptureFollowUpBroadcastMode::ShutdownFlush,
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

    let worker = ProxyCaptureFollowUpWorker {
        latest_broadcast_seq: state.proxy_summary_quota_broadcast_seq.clone(),
        broadcast_running: state.proxy_summary_quota_broadcast_running.clone(),
        pool: state.pool.clone(),
        broadcaster: state.broadcaster.clone(),
        broadcast_state_cache: state.broadcast_state_cache.clone(),
        subscription_hub: state.subscription_hub.clone(),
        shutdown: state.shutdown.clone(),
        invoke_id: invoke_id.to_string(),
    };
    let broadcast_handle_slot = state.proxy_summary_quota_broadcast_handle.clone();
    let handle = spawn_proxy_capture_follow_up_worker(worker);

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
            error!(?err, "quota broadcast worker terminated unexpectedly");
        }
    }

    Ok(())
}

struct ProxyCaptureFollowUpWorker {
    latest_broadcast_seq: Arc<AtomicU64>,
    broadcast_running: Arc<AtomicBool>,
    pool: Pool<Sqlite>,
    broadcaster: broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: Arc<Mutex<BroadcastStateCache>>,
    subscription_hub: Arc<SubscriptionHub>,
    shutdown: CancellationToken,
    invoke_id: String,
}

fn spawn_proxy_capture_follow_up_worker(
    worker: ProxyCaptureFollowUpWorker,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut synced_seq = 0_u64;
        loop {
            let target_seq = worker.latest_broadcast_seq.load(Ordering::Acquire);
            if worker.shutdown.is_cancelled() {
                if target_seq != synced_seq {
                    broadcast_proxy_capture_follow_up(
                        &worker.pool,
                        &worker.broadcaster,
                        worker.broadcast_state_cache.as_ref(),
                        worker.subscription_hub.as_ref(),
                        ProxyCaptureFollowUpBroadcastMode::ShutdownFlush,
                        &worker.invoke_id,
                    )
                    .await;
                }
                worker.broadcast_running.store(false, Ordering::Release);
                break;
            }
            if target_seq == synced_seq {
                let should_continue = finish_summary_quota_broadcast_idle(
                    SummaryQuotaBroadcastIdleContext {
                        latest_broadcast_seq: worker.latest_broadcast_seq.as_ref(),
                        broadcast_running: worker.broadcast_running.as_ref(),
                        shutdown: &worker.shutdown,
                        pool: &worker.pool,
                        broadcaster: &worker.broadcaster,
                        broadcast_state_cache: worker.broadcast_state_cache.as_ref(),
                        subscription_hub: worker.subscription_hub.as_ref(),
                        invoke_id: &worker.invoke_id,
                    },
                    synced_seq,
                )
                .await;
                if should_continue {
                    continue;
                }
                break;
            }
            synced_seq = target_seq;
            tokio::select! {
                _ = worker.shutdown.cancelled() => {
                    broadcast_proxy_capture_follow_up(
                        &worker.pool,
                        &worker.broadcaster,
                        worker.broadcast_state_cache.as_ref(),
                        worker.subscription_hub.as_ref(),
                        ProxyCaptureFollowUpBroadcastMode::ShutdownFlush,
                        &worker.invoke_id,
                    ).await;
                    worker.broadcast_running.store(false, Ordering::Release);
                    break;
                }
                _ = broadcast_proxy_capture_follow_up(
                    &worker.pool,
                    &worker.broadcaster,
                    worker.broadcast_state_cache.as_ref(),
                    worker.subscription_hub.as_ref(),
                    ProxyCaptureFollowUpBroadcastMode::ActiveSubscribers,
                    &worker.invoke_id,
                ) => {}
            }
        }
    })
}

pub(crate) fn schedule_proxy_capture_follow_up_after_terminal_enqueue(
    state: &AppState,
    invoke_id: &str,
    trigger: &'static str,
) {
    #[cfg(test)]
    if !state.sqlite_batch_writer.auto_flush_terminal_for_test() {
        return;
    }

    if !state.shutdown.is_cancelled()
        && !state
            .subscription_hub
            .has_active_topic_name_sync("quota.current")
    {
        let subscription_hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            subscription_hub
                .mark_topic_name_dirty("quota.current")
                .await;
        });
        return;
    }

    let pool = state.pool.clone();
    let broadcaster = state.broadcaster.clone();
    let broadcast_state_cache = state.broadcast_state_cache.clone();
    let subscription_hub = state.subscription_hub.clone();
    let shutdown = state.shutdown.clone();
    let invoke_id = invoke_id.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mode = if shutdown.is_cancelled() {
            ProxyCaptureFollowUpBroadcastMode::ShutdownFlush
        } else {
            ProxyCaptureFollowUpBroadcastMode::ActiveSubscribers
        };
        debug!(
            invoke_id = %invoke_id,
            trigger,
            record_flush_deferred_or_failed = "deferred_follow_up_without_forced_sqlite_barrier",
            "proxy capture follow-up deferred without forcing terminal sqlite flush"
        );
        broadcast_proxy_capture_follow_up(
            &pool,
            &broadcaster,
            broadcast_state_cache.as_ref(),
            subscription_hub.as_ref(),
            mode,
            &invoke_id,
        )
        .await;
    });
}
