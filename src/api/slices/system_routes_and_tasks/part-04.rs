pub(crate) fn spawn_system_raw_payload_metrics_inventory(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(error) =
                crate::resume_retention_raw_payload_metrics_inventory_reset(state.as_ref()).await
            {
                set_system_raw_metrics_health_override(state.as_ref(), Some("error")).await;
                warn!(error = %error, "system raw metrics inventory reset resume failed");
            }
            if let Err(error) = refresh_system_raw_payload_metrics_inventory(state.as_ref()).await {
                set_system_raw_metrics_health_override(state.as_ref(), Some("error")).await;
                warn!(error = %error, "system raw metrics inventory batch failed");
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
        }
    })
}

async fn await_system_status_refresh_operation<T>(
    cancellation: &CancellationToken,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => bail!("system status snapshot refresh cancelled"),
        result = operation => result,
    }
}

struct SystemStatusSnapshotData {
    runtime_pressure_health: SystemRuntimePressureHealth,
    invocation_status: SystemInvocationStatusAggRow,
    archived: SystemArchiveAggRow,
    raw_metrics: SystemRawPayloadMetricsRow,
    raw_metrics_state: String,
    filesystem_bytes: SystemStatusFilesystemBytes,
    terminal_health: crate::terminal_projection::TerminalProjectionHealth,
    long_term_health: crate::long_term_stats::LongTermProjectionHealth,
}

fn build_system_status_response(data: SystemStatusSnapshotData) -> SystemStatusResponse {
    let SystemStatusSnapshotData {
        runtime_pressure_health,
        invocation_status,
        archived,
        raw_metrics,
        raw_metrics_state,
        filesystem_bytes,
        terminal_health,
        long_term_health,
    } = data;
    SystemStatusResponse {
        live_invocations_count: invocation_status.live_invocations_count.unwrap_or(0).max(0) as u64,
        success_count: invocation_status.success_count.unwrap_or(0).max(0) as u64,
        non_success_count: invocation_status.non_success_count.unwrap_or(0).max(0) as u64,
        completed_archive_batches_count: archived
            .completed_archive_batches_count
            .unwrap_or(0)
            .max(0) as u64,
        archived_bodies: SystemStatusMetric {
            count: archived.archived_count.unwrap_or(0).max(0) as u64,
            bytes: filesystem_bytes.archive_bytes,
        },
        raw_bodies: SystemStatusMetric {
            count: raw_metrics.raw_count.max(0) as u64,
            bytes: raw_metrics.raw_bytes.max(0) as u64,
        },
        request_raw_bodies: SystemStatusMetric {
            count: raw_metrics.request_raw_count.max(0) as u64,
            bytes: raw_metrics.request_raw_bytes.max(0) as u64,
        },
        response_raw_bodies: SystemStatusMetric {
            count: raw_metrics.response_raw_count.max(0) as u64,
            bytes: raw_metrics.response_raw_bytes.max(0) as u64,
        },
        database_bytes: filesystem_bytes.database_bytes,
        other_files_bytes: filesystem_bytes.other_files_bytes,
        projection_health: SystemProjectionHealth {
            terminal: SystemProjectionConsumerHealth {
                state: if terminal_health.dirty_last_good {
                    "dirty_last_good".to_string()
                } else {
                    "healthy".to_string()
                },
                cursor_lag: terminal_health
                    .last_persisted_row_id
                    .saturating_sub(terminal_health.long_term_cursor_row_id),
                dirty_bucket_count: 0,
                pending_event_count: terminal_health.pending_event_count as u64,
                last_flush_elapsed_ms: None,
                last_flush_age_ms: terminal_health.last_ack_age_ms,
                last_repair_scope: None,
                last_defer_reason: terminal_health.hard_limit_reason.map(str::to_string),
                last_error_kind: None,
            },
            long_term: SystemProjectionConsumerHealth {
                state: long_term_health.state,
                cursor_lag: terminal_health
                    .last_persisted_row_id
                    .saturating_sub(long_term_health.cursor_row_id),
                dirty_bucket_count: long_term_health.dirty_bucket_count as u64,
                pending_event_count: long_term_health.pending_event_count as u64,
                last_flush_elapsed_ms: long_term_health.last_flush_elapsed_ms,
                last_flush_age_ms: long_term_health.last_flush_age_ms,
                last_repair_scope: long_term_health.last_repair_scope,
                last_defer_reason: long_term_health.last_defer_reason,
                last_error_kind: long_term_health.last_error_kind,
            },
        },
        raw_metrics_health: SystemRawMetricsHealth {
            state: raw_metrics_state,
            inventory_cursor: raw_metrics.inventory_cursor,
            updated_age_ms: None,
        },
        runtime_pressure_health: Some(runtime_pressure_health),
        refreshed_at: format_utc_iso(Utc::now()),
    }
}

async fn load_system_status_database_counts(
    state: &AppState,
    cancellation: &CancellationToken,
) -> Result<(SystemInvocationStatusAggRow, SystemArchiveAggRow)> {
    let invocation_status = await_system_status_refresh_operation(cancellation, async {
        Ok(sqlx::query_as::<_, SystemInvocationStatusAggRow>(
        r#"
        SELECT
            COUNT(*) AS live_invocations_count,
            COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'warning_success') THEN 1 ELSE 0 END), 0) AS success_count,
            COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) NOT IN ('success', 'warning_success') THEN 1 ELSE 0 END), 0) AS non_success_count
        FROM codex_invocations
        "#,
    )
    .fetch_one(&state.pool)
    .await?)
    })
    .await?;
    let archived = await_system_status_refresh_operation(cancellation, async {
        Ok(sqlx::query_as::<_, SystemArchiveAggRow>(
            r#"
        SELECT
            COUNT(*) AS completed_archive_batches_count,
            COALESCE(SUM(row_count), 0) AS archived_count
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
        "#,
        )
        .fetch_one(&state.pool)
        .await?)
    })
    .await?;
    Ok((invocation_status, archived))
}

async fn load_system_status_snapshot_uncached(
    state: &AppState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(SystemStatusResponse, String)> {
    let runtime_pressure_health = await_system_status_refresh_operation(cancellation, async {
        Ok(load_runtime_pressure_health(state).await)
    })
    .await?;
    let (invocation_status, archived) =
        load_system_status_database_counts(state, cancellation).await?;

    let raw_metrics = await_system_status_refresh_operation(cancellation, async {
        Ok(sqlx::query_as::<_, SystemRawPayloadMetricsRow>(
        "SELECT inventory_state, inventory_cursor, link_inventory_cursor, raw_count, raw_bytes, request_raw_count, request_raw_bytes, response_raw_count, response_raw_bytes, updated_at FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&state.pool)
    .await?)
    })
    .await?;
    let raw_metrics_state = await_system_status_refresh_operation(cancellation, async {
        Ok(state
            .system_status_cache
            .lock()
            .await
            .raw_metrics_health_override
            .clone()
            .unwrap_or_else(|| raw_metrics.inventory_state.clone()))
    })
    .await?;

    let archive_dir = resolved_archive_dir(&state.config);
    let raw_dir = state.config.resolved_proxy_raw_dir();
    let archived_paths = await_system_status_refresh_operation(cancellation, async {
        Ok(sqlx::query_scalar::<_, String>(
            r#"
        SELECT file_path
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
        "#,
        )
        .fetch_all(&state.pool)
        .await?)
    })
    .await?;
    let filesystem_bytes = collect_system_status_filesystem_bytes_in_blocking_task(
        archived_paths,
        state.config.clone(),
        archive_dir,
        raw_dir,
        &state.system_status_cache,
        cancellation,
        deadline,
    )
    .await?;
    let terminal_health = state.terminal_projection_hub.health();
    let long_term_health = await_system_status_refresh_operation(cancellation, async {
        Ok(state.long_term_projection_runtime.lock().await.health())
    })
    .await?;
    let runtime_record_count = state.proxy_runtime_invocations.runtime_record_count() as u64;
    debug!(
        db_invocation_row_count = invocation_status.live_invocations_count.unwrap_or(0).max(0),
        runtime_record_count,
        "system status invocation counts keep database rows separate from runtime memory records"
    );
    let raw_metrics_inventory_state = raw_metrics.inventory_state.clone();
    Ok((
        build_system_status_response(SystemStatusSnapshotData {
            runtime_pressure_health,
            invocation_status,
            archived,
            raw_metrics,
            raw_metrics_state,
            filesystem_bytes,
            terminal_health,
            long_term_health,
        }),
        raw_metrics_inventory_state,
    ))
}

pub(crate) async fn load_system_status_uncached(state: &AppState) -> Result<SystemStatusResponse> {
    let cancellation = state.shutdown.child_token();
    Ok(load_system_status_snapshot_uncached(
        state,
        &cancellation,
        Instant::now() + SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE,
    )
    .await?
    .0)
}

pub(crate) async fn load_system_status_cached(state: &AppState) -> Result<SystemStatusResponse> {
    if let Some((response, snapshot_age)) = state
        .system_status_cache
        .lock()
        .await
        .latest
        .as_ref()
        .map(|entry| (entry.response.clone(), entry.cached_at.elapsed()))
        .filter(|(_, snapshot_age)| {
            *snapshot_age <= Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS)
        })
    {
        let snapshot_age_ms = snapshot_age.as_millis() as u64;
        debug!(
            metrics_source = "system_status_memory_snapshot",
            snapshot_age_ms, "serving system status from last-good memory snapshot"
        );
        return Ok(response);
    }

    Err(anyhow!("system status snapshot is unavailable or stale"))
}

pub(crate) async fn invalidate_system_status_cache(state: &AppState) {
    // A failed background refresh must never turn a previously good response into an empty
    // request-side cache miss. The maintainer will refresh this last-good entry on its cadence.
    let cache = state.system_status_cache.lock().await;
    debug!(
        has_last_good = cache.latest.is_some(),
        "system status snapshot marked for background refresh"
    );
}

pub(crate) async fn hydrate_system_status_snapshot(state: &AppState) -> Result<()> {
    refresh_system_status_snapshot_with_deadline(state).await
}

async fn refresh_system_status_snapshot_with_deadline(state: &AppState) -> Result<()> {
    let cancellation = state.shutdown.child_token();
    let deadline = Instant::now() + SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE;
    let refresh = refresh_system_status_snapshot_with_cancellation(state, &cancellation, deadline);
    tokio::pin!(refresh);

    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            cancellation.cancel();
            let _ = refresh.await;
            bail!("system status snapshot refresh cancelled");
        }
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            cancellation.cancel();
            let _ = refresh.await;
            bail!("system status snapshot refresh exceeded its deadline");
        }
        result = &mut refresh => result,
    }
}

async fn refresh_system_status_snapshot(state: &AppState) -> Result<()> {
    let cancellation = state.shutdown.child_token();
    refresh_system_status_snapshot_with_cancellation(
        state,
        &cancellation,
        Instant::now() + SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE,
    )
    .await
}

#[derive(Debug)]
struct SystemStatusSnapshotRefreshFlight {
    cache: Arc<Mutex<SystemStatusCacheState>>,
}

impl SystemStatusSnapshotRefreshFlight {
    async fn begin(state: &AppState, cancellation: &CancellationToken) -> Result<Self> {
        let mut cache = await_system_status_refresh_operation(cancellation, async {
            Ok(state.system_status_cache.lock().await)
        })
        .await?;
        if cache.in_flight.is_some() {
            bail!("system status snapshot refresh is already in flight");
        }
        let (signal, _receiver) = watch::channel(false);
        cache.in_flight = Some(signal);
        Ok(Self {
            cache: state.system_status_cache.clone(),
        })
    }

    async fn finish(self) {
        let mut cache = self.cache.lock().await;
        if let Some(signal) = cache.in_flight.take() {
            let _ = signal.send(true);
        }
    }
}

async fn refresh_system_status_snapshot_with_cancellation(
    state: &AppState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<()> {
    let flight = SystemStatusSnapshotRefreshFlight::begin(state, cancellation).await?;
    let result = async {
        let (response, raw_metrics_inventory_state) =
            load_system_status_snapshot_uncached(state, cancellation, deadline).await?;
        publish_system_status_snapshot(
            state,
            cancellation,
            deadline,
            response,
            raw_metrics_inventory_state,
        )
        .await
    }
    .await;
    flight.finish().await;
    result
}

fn ensure_system_status_snapshot_refresh_active(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<()> {
    if cancellation.is_cancelled() {
        bail!("system status snapshot refresh cancelled");
    }
    if Instant::now() >= deadline {
        cancellation.cancel();
        bail!("system status snapshot refresh exceeded its deadline");
    }
    Ok(())
}

async fn publish_system_status_snapshot(
    state: &AppState,
    cancellation: &CancellationToken,
    deadline: Instant,
    mut response: SystemStatusResponse,
    raw_metrics_inventory_state: String,
) -> Result<()> {
    await_system_status_refresh_operation(cancellation, async {
        let mut cache = state.system_status_cache.lock().await;
        // The cache lock can be delayed past the refresh deadline. Recheck while holding it so a
        // completed filesystem scan cannot publish a stale snapshot.
        ensure_system_status_snapshot_refresh_active(cancellation, deadline)?;
        if let Some(override_state) = cache.raw_metrics_health_override.as_deref() {
            response.raw_metrics_health.state = override_state.to_string();
        }
        cache.latest = Some(SystemStatusCacheEntry {
            cached_at: Instant::now(),
            response,
            raw_metrics_inventory_state,
        });
        debug!(
            metrics_source = "system_status_memory_snapshot",
            cache_ttl_ms = SYSTEM_STATUS_CACHE_TTL_SECS * 1_000,
            "system status background snapshot refresh completed"
        );
        Ok(())
    })
    .await
}

fn system_status_snapshot_refresh_delay() -> Duration {
    Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS)
        .checked_sub(SYSTEM_STATUS_SNAPSHOT_REFRESH_LEAD)
        .expect("system status refresh lead must be shorter than the cache TTL")
}

fn system_status_snapshot_refresh_cadence_period() -> Duration {
    // Reapply the lead after every publication. A TTL-sized period only protects the first
    // scan and eventually schedules later refreshes at a last-good entry's expiry boundary.
    system_status_snapshot_refresh_delay()
}

pub(crate) fn spawn_system_status_snapshot_maintenance(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Startup has already completed the first durable hydration before this producer is
        // spawned. Keep every bounded scan ahead of the public TTL rather than only advancing the
        // first tick; a 60-second period would eventually start a scan at the prior snapshot's
        // expiry boundary when a preceding scan completed after its scheduled tick.
        let mut cadence = tokio::time::interval_at(
            tokio::time::Instant::now() + system_status_snapshot_refresh_delay(),
            system_status_snapshot_refresh_cadence_period(),
        );
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = cadence.tick() => {}
            }
            if let Err(error) = refresh_system_status_snapshot_with_deadline(state.as_ref()).await {
                warn!(
                    ?error,
                    "system status background refresh failed; retaining last-good snapshot"
                );
            }
        }
    });
}
