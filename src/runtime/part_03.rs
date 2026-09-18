async fn finish_orphaned_startup_hourly_rollup_bootstrap_task(
    state: &AppState,
    cancel: &CancellationToken,
    started_at_from: &str,
) {
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        let task = tokio::time::timeout(
            Duration::from_millis(50),
            sqlx::query_as::<_, (i64, String)>(
                r#"
                SELECT id, trigger_kind
                FROM system_task_runs
                WHERE task_kind = ?1
                  AND trigger_kind = 'startup'
                  AND status = ?2
                  AND summary = 'background hourly rollup bootstrap started'
                  AND started_at >= ?3
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .bind(SystemTaskKind::HourlyRollupBootstrap.as_str())
            .bind(SystemTaskStatus::Running.as_str())
            .bind(started_at_from)
            .fetch_optional(&state.pool),
        )
        .await;
        match task {
            Ok(Ok(Some((id, trigger_kind)))) => {
                let task_run = SystemTaskRunHandle {
                    id,
                    task_kind: SystemTaskKind::HourlyRollupBootstrap,
                    trigger_kind,
                    started_at: Instant::now(),
                };
                finish_runtime_startup_hourly_rollup_bootstrap_task(
                    state,
                    cancel,
                    Some(&task_run),
                    SystemTaskStatus::Skipped,
                    "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
                    None,
                )
                .await;
                return;
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(err)) => {
                debug!(error = %err, "failed to inspect for an orphaned startup hourly rollup bootstrap task");
                return;
            }
        }
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(crate) async fn finish_runtime_startup_hourly_rollup_bootstrap_task(
    state: &AppState,
    cancel: &CancellationToken,
    task_run: Option<&SystemTaskRunHandle>,
    status: SystemTaskStatus,
    summary: &str,
    detail: Option<String>,
) {
    if let Some(task_run) = task_run {
        let finished = finish_system_task_run_reliably(
            state,
            Some(cancel),
            task_run,
            status,
            Some(summary.to_string()),
            detail,
        )
        .await;
        if !finished {
            warn!(
                task_kind = task_run.task_kind.as_str(),
                trigger_kind = %task_run.trigger_kind,
                timeout_ms = STARTUP_HOURLY_ROLLUP_BOOTSTRAP_CANCELLED_TASK_FINISH_TIMEOUT.as_millis() as u64,
                "failed to durably finalize startup hourly rollup bootstrap task-history after bounded retries"
            );
        }
    }
}

async fn refresh_forward_proxy_subscriptions_at_startup(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
) -> bool {
    let startup_known_subscription_keys = {
        let manager = state.forward_proxy.lock().await;
        snapshot_known_subscription_proxy_keys(&manager)
    };
    if cancel.is_cancelled() {
        info!("forward proxy maintenance skipped because shutdown is already in progress");
        return false;
    }
    let startup_run = tokio::select! {
        biased;
        _ = cancel.cancelled() => return false,
        result = begin_system_task_run_admitted(
            state.as_ref(),
            crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
            SystemTaskKind::ForwardProxySubscriptionRefresh,
            "startup",
            Some("forward proxy subscription refresh started".to_string()),
        ) => result.ok(),
    };
    if let Err(err) = refresh_forward_proxy_subscriptions(
        state.clone(),
        true,
        Some(startup_known_subscription_keys),
    )
    .await
    {
        if let Some(run) = startup_run.as_ref() {
            let _ = finish_system_task_run_reliably(
                state.as_ref(),
                Some(&cancel),
                run,
                SystemTaskStatus::Failed,
                Some("forward proxy startup refresh failed".to_string()),
                Some(err.to_string()),
            )
            .await;
        }
        warn!(error = %err, "failed to refresh forward proxy subscriptions at startup");
    } else if let Some(run) = startup_run.as_ref() {
        let _ = finish_system_task_run_reliably(
            state.as_ref(),
            Some(&cancel),
            run,
            SystemTaskStatus::Success,
            Some("forward proxy startup refresh completed".to_string()),
            None,
        )
        .await;
    }

    true
}

async fn maintain_forward_proxy_subscriptions(state: Arc<AppState>, cancel: CancellationToken) {
    if !refresh_forward_proxy_subscriptions_at_startup(&state, &cancel).await {
        return;
    }
    let mut ticker = interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("forward proxy maintenance received shutdown");
                break;
            }
            _ = ticker.tick() => {
                let task_run = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    result = begin_system_task_run_admitted(
                        state.as_ref(),
                        crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived,
                        SystemTaskKind::ForwardProxySubscriptionRefresh,
                        "interval",
                        Some("forward proxy interval refresh started".to_string()),
                    ) => result.ok(),
                };
                if let Err(err) = refresh_forward_proxy_subscriptions(state.clone(), false, None).await {
                    if let Some(run) = task_run.as_ref() {
                        let _ = finish_system_task_run_reliably(
                            state.as_ref(),
                            Some(&cancel),
                            run,
                            SystemTaskStatus::Failed,
                            Some("forward proxy interval refresh failed".to_string()),
                            Some(err.to_string()),
                        )
                        .await;
                    }
                    warn!(error = %err, "failed to refresh forward proxy subscriptions");
                } else if let Some(run) = task_run.as_ref() {
                    let _ = finish_system_task_run_reliably(
                        state.as_ref(),
                        Some(&cancel),
                        run,
                        SystemTaskStatus::Success,
                        Some("forward proxy interval refresh completed".to_string()),
                        None,
                    )
                    .await;
                }
                if let Err(err) = flush_dashboard_network_socket_minute_rollups(
                    &state.pool,
                    state.dashboard_network_speed_cache.as_ref(),
                    Utc::now(),
                )
                .await
                {
                    warn!(error = %err, "failed to flush dashboard socket minute rollups");
                }
            }
        }
    }
}

pub(crate) fn spawn_forward_proxy_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(maintain_forward_proxy_subscriptions(state, cancel))
}

pub(crate) fn spawn_pool_orphan_recovery_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(POOL_EARLY_PHASE_ORPHAN_RECOVERY_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("pool orphan recovery maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    match recover_stale_pool_early_phase_orphans_runtime(state.as_ref()).await {
                        Ok(outcome) => {
                            if outcome.recovered_attempts > 0 || outcome.recovered_invocations > 0 {
                                warn!(
                                    recovered_attempts = outcome.recovered_attempts,
                                    recovered_invocations = outcome.recovered_invocations,
                                    "runtime pool orphan recovery swept stale early-phase rows"
                                );
                            }
                        }
                        Err(err) => {
                            warn!(error = %err, "failed to recover stale pool early-phase orphans at runtime");
                        }
                    }
                }
            }
        }
    })
}
