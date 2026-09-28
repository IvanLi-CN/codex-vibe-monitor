use super::*;

pub(crate) async fn refresh_system_raw_payload_metrics_inventory(state: &AppState) -> Result<()> {
    let memory_baseline = state.memory_diagnostics.begin_operation(state).await;
    let result = refresh_system_raw_payload_metrics_inventory_inner(state).await;
    let load_row_count = result.as_ref().copied().unwrap_or_default();
    state
        .memory_diagnostics
        .observe_operation(
            state,
            "system_raw_payload_metrics_inventory",
            memory_baseline,
            load_row_count,
            true,
        )
        .await;
    if result.is_err() {
        state.raw_capture_circuit.mark_inventory_preparing();
        set_system_raw_metrics_health_override(state, Some("error")).await;
    }
    result.map(|_| ())
}

pub(crate) async fn hydrate_raw_capture_circuit(state: &AppState) -> Result<()> {
    let row = sqlx::query_as::<_, SystemRawPayloadMetricsRow>(
        "SELECT inventory_state, inventory_cursor, link_inventory_cursor, inventory_recheck_cursor, inventory_recheck_active, raw_count, raw_bytes, raw_overflow_spool_bytes, request_raw_count, request_raw_bytes, response_raw_count, response_raw_bytes, circuit_state, circuit_reason, circuit_available_bytes, circuit_expired_backlog_count, circuit_backlog_non_growing, circuit_updated_at, circuit_recovery_pending, updated_at FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&state.pool)
    .await?;
    state.raw_capture_circuit.hydrate_with_recovery_pending(
        &row.inventory_state,
        Some(&row.circuit_state),
        row.circuit_reason.as_deref(),
        row.raw_bytes.max(0) as u64,
        Some(row.raw_overflow_spool_bytes.max(0) as u64),
        row.circuit_available_bytes.map(|value| value.max(0) as u64),
        row.circuit_expired_backlog_count
            .map(|value| value.max(0) as u64),
        row.circuit_backlog_non_growing.map(|value| value != 0),
        row.circuit_recovery_pending != 0,
        row.circuit_updated_at,
    );
    if row.inventory_state != "resetting" {
        sqlx::query(
            "UPDATE system_raw_payload_metrics SET inventory_state = 'preparing', inventory_recheck_cursor = '', inventory_recheck_active = 1, updated_at = datetime('now') WHERE singleton = 1 AND inventory_state != 'resetting'",
        )
        .execute(&state.pool)
        .await?;
    }
    Ok(())
}

async fn persist_raw_capture_circuit(state: &AppState) -> Result<()> {
    let snapshot = state.raw_capture_circuit.snapshot();
    sqlx::query(
        r#"
        UPDATE system_raw_payload_metrics
        SET circuit_state = ?1,
            circuit_reason = ?2,
            raw_bytes = COALESCE(?3, raw_bytes),
            raw_overflow_spool_bytes = COALESCE(?4, raw_overflow_spool_bytes),
            circuit_available_bytes = ?5,
            circuit_expired_backlog_count = ?6,
            circuit_backlog_non_growing = ?7,
            circuit_recovery_pending = ?8,
            circuit_updated_at = ?9
        WHERE singleton = 1
          AND inventory_state != 'resetting'
        "#,
    )
    .bind(snapshot.state)
    .bind(snapshot.reason)
    .bind(snapshot.raw_bytes.map(|value| value as i64))
    .bind(snapshot.spool_bytes.map(|value| value as i64))
    .bind(snapshot.available_bytes.map(|value| value as i64))
    .bind(snapshot.expired_backlog_count.map(|value| value as i64))
    .bind(snapshot.backlog_non_growing.map(i64::from))
    .bind(i64::from(snapshot.recovery_pending))
    .bind(snapshot.updated_at)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn refresh_system_raw_payload_metrics_inventory_inner(state: &AppState) -> Result<u64> {
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _permit = match gate.try_begin_background("system_raw_metrics_inventory") {
        Ok(permit) => permit,
        Err(reason) => {
            set_system_raw_metrics_health_override(state, Some("deferred")).await;
            debug!(
                metrics_source = "inventory",
                gate_outcome = "deferred",
                defer_reason = "writer_pressure",
                reason = %reason,
                "system raw metrics inventory deferred by database pressure"
            );
            return Ok(0);
        }
    };
    let (inventory_generation, accounting_generation, _) =
        state.raw_capture_circuit.inventory_checkpoint();
    let snapshot = sqlx::query_as::<_, SystemRawPayloadMetricsRow>(
        "SELECT inventory_state, inventory_cursor, link_inventory_cursor, inventory_recheck_cursor, inventory_recheck_active, raw_count, raw_bytes, raw_overflow_spool_bytes, request_raw_count, request_raw_bytes, response_raw_count, response_raw_bytes, circuit_state, circuit_reason, circuit_available_bytes, circuit_expired_backlog_count, circuit_backlog_non_growing, circuit_updated_at, circuit_recovery_pending, updated_at FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&state.pool)
    .await?;
    if snapshot.inventory_state == "resetting" {
        set_system_raw_metrics_health_override(state, Some("preparing")).await;
        debug!(
            metrics_source = "inventory",
            "system raw metrics inventory is waiting for retention reset batches"
        );
        return Ok(0);
    }
    let rows = sqlx::query_as::<_, SystemRawPayloadInventoryRow>(
        r#"
        SELECT id, request_raw_path, response_raw_path
        FROM codex_invocations
        WHERE id > ?1
          AND (request_raw_path IS NOT NULL OR response_raw_path IS NOT NULL)
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(snapshot.inventory_cursor)
    .bind(SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE)
    .fetch_all(&state.pool)
    .await?;
    let link_rows = sqlx::query_as::<_, SystemRawPayloadBlobLinkRow>(
        r#"
        SELECT id, raw_path, raw_role
        FROM proxy_raw_payload_blob_links
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(snapshot.link_inventory_cursor)
    .bind(SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE)
    .fetch_all(&state.pool)
    .await?;
    let recheck_rows = if snapshot.inventory_recheck_active != 0 {
        sqlx::query_as::<_, SystemRawPayloadInventoryRecheckRow>(
            r#"
            SELECT raw_path, byte_size, request_seen, response_seen
            FROM system_raw_payload_inventory_paths
            WHERE raw_path > ?1
            ORDER BY raw_path ASC
            LIMIT ?2
            "#,
        )
        .bind(&snapshot.inventory_recheck_cursor)
        .bind(SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE)
        .fetch_all(&state.pool)
        .await?
    } else {
        Vec::new()
    };

    let fallback_root = state.config.database_path.parent();
    let mut paths = HashMap::<String, (i64, bool, bool)>::new();
    for row in &rows {
        for (raw_path, is_request) in [
            (row.request_raw_path.as_deref(), true),
            (row.response_raw_path.as_deref(), false),
        ] {
            let Some(raw_path) = raw_path else {
                continue;
            };
            let Some(candidate) = resolved_raw_path_read_candidates(raw_path, fallback_root)
                .into_iter()
                .find(|candidate| candidate.exists())
            else {
                continue;
            };
            let entry = paths
                .entry(candidate.to_string_lossy().to_string())
                .or_insert((count_file_size(&candidate) as i64, false, false));
            if is_request {
                entry.1 = true;
            } else {
                entry.2 = true;
            }
        }
    }
    for row in &link_rows {
        let Some(candidate) = resolved_raw_path_read_candidates(&row.raw_path, fallback_root)
            .into_iter()
            .find(|candidate| candidate.exists())
        else {
            continue;
        };
        let entry = paths
            .entry(candidate.to_string_lossy().to_string())
            .or_insert((count_file_size(&candidate) as i64, false, false));
        match row.raw_role.as_str() {
            "request" => entry.1 = true,
            "response" => entry.2 = true,
            _ => {}
        }
    }

    let spool_directory = state
        .config
        .resolved_proxy_raw_dir()
        .join(crate::proxy::RAW_OVERFLOW_SPOOL_DIR);
    let (spool_bytes, spool_inventory_overflow) =
        match crate::proxy::bounded_raw_overflow_spool_directory_bytes(
            &spool_directory,
            SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE as usize,
        ) {
            Ok(result) => result,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (0, false),
            Err(error) => {
                return Err(error).context("failed to inspect raw overflow spool inventory");
            }
        };
    state
        .raw_capture_circuit
        .set_inventory_spool_overflow(spool_inventory_overflow);
    if !spool_inventory_overflow {
        crate::proxy::set_raw_overflow_spool_accounted_bytes(&spool_directory, spool_bytes);
    }

    let recheck_changes = recheck_rows
        .iter()
        .filter_map(|row| {
            let current_path = resolved_raw_path_read_candidates(&row.raw_path, fallback_root)
                .into_iter()
                .find(|candidate| candidate.exists());
            let current_size = current_path
                .as_ref()
                .map(|candidate| count_file_size(candidate) as i64)
                .unwrap_or_default();
            (current_size != row.byte_size).then_some((
                row.raw_path.clone(),
                current_size,
                current_size.saturating_sub(row.byte_size),
                current_path.is_some(),
            ))
        })
        .collect::<Vec<_>>();
    let mut deltas = (0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
    let mut tx = state.pool.begin().await?;
    for (path, (byte_size, request_seen, response_seen)) in paths {
        let delta = record_system_raw_payload_inventory_path(
            &mut tx,
            &path,
            byte_size,
            request_seen,
            response_seen,
        )
        .await?;
        deltas.0 += delta.0;
        deltas.1 += delta.1;
        deltas.2 += delta.2;
        deltas.3 += delta.3;
        deltas.4 += delta.4;
        deltas.5 += delta.5;
    }
    for (path, byte_size, delta, current_present) in recheck_changes {
        sqlx::query(
            "UPDATE system_raw_payload_inventory_paths SET byte_size = ?2 WHERE raw_path = ?1",
        )
        .bind(&path)
        .bind(byte_size)
        .execute(tx.as_mut())
        .await?;
        let flags = sqlx::query_as::<_, SystemRawPayloadInventoryPathRow>(
            "SELECT byte_size, request_seen, response_seen FROM system_raw_payload_inventory_paths WHERE raw_path = ?1",
        )
        .bind(&path)
        .fetch_one(tx.as_mut())
        .await?;
        if current_present && byte_size > 0 && delta == byte_size {
            deltas.0 = deltas.0.saturating_add(1);
            if flags.request_seen != 0 {
                deltas.2 = deltas.2.saturating_add(1);
            }
            if flags.response_seen != 0 {
                deltas.4 = deltas.4.saturating_add(1);
            }
        } else if !current_present && delta < 0 {
            deltas.0 = deltas.0.saturating_sub(1);
            if flags.request_seen != 0 {
                deltas.2 = deltas.2.saturating_sub(1);
            }
            if flags.response_seen != 0 {
                deltas.4 = deltas.4.saturating_sub(1);
            }
        }
        deltas.1 = deltas.1.saturating_add(delta);
        if flags.request_seen != 0 {
            deltas.3 = deltas.3.saturating_add(delta);
        }
        if flags.response_seen != 0 {
            deltas.5 = deltas.5.saturating_add(delta);
        }
    }
    let next_cursor = rows
        .last()
        .map(|row| row.id)
        .unwrap_or(snapshot.inventory_cursor);
    let next_link_cursor = link_rows
        .last()
        .map(|row| row.id)
        .unwrap_or(snapshot.link_inventory_cursor);
    let recheck_active = snapshot.inventory_recheck_active != 0
        && recheck_rows.len() == SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE as usize;
    let next_recheck_cursor = if recheck_active {
        recheck_rows
            .last()
            .map(|row| row.raw_path.clone())
            .unwrap_or_else(|| snapshot.inventory_recheck_cursor.clone())
    } else {
        String::new()
    };
    let state_name = if !spool_inventory_overflow
        && !recheck_active
        && rows.len() < SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE as usize
        && link_rows.len() < SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE as usize
    {
        "ready"
    } else {
        "preparing"
    };
    let update_result = sqlx::query(
        r#"
        UPDATE system_raw_payload_metrics
        SET inventory_state = ?1,
            inventory_cursor = ?2,
            link_inventory_cursor = ?3,
            inventory_recheck_cursor = ?4,
            inventory_recheck_active = ?5,
            raw_count = raw_count + ?6,
            raw_bytes = raw_bytes + ?7,
            request_raw_count = request_raw_count + ?8,
            request_raw_bytes = request_raw_bytes + ?9,
            response_raw_count = response_raw_count + ?10,
            response_raw_bytes = response_raw_bytes + ?11,
            raw_overflow_spool_bytes = ?12,
            updated_at = datetime('now')
        WHERE singleton = 1
          AND inventory_state = ?13
        "#,
    )
    .bind(state_name)
    .bind(next_cursor)
    .bind(next_link_cursor)
    .bind(next_recheck_cursor)
    .bind(i64::from(recheck_active))
    .bind(deltas.0)
    .bind(deltas.1)
    .bind(deltas.2)
    .bind(deltas.3)
    .bind(deltas.4)
    .bind(deltas.5)
    .bind(Some(spool_bytes as i64))
    .bind(&snapshot.inventory_state)
    .execute(tx.as_mut())
    .await?;
    if update_result.rows_affected() == 0 {
        tx.rollback().await?;
        set_system_raw_metrics_health_override(state, Some("preparing")).await;
        return Ok(0);
    }
    tx.commit().await?;
    let next_raw_bytes = if deltas.1 >= 0 {
        snapshot.raw_bytes.saturating_add(deltas.1) as u64
    } else {
        snapshot
            .raw_bytes
            .saturating_sub(deltas.1.unsigned_abs().min(i64::MAX as u64) as i64) as u64
    };
    let retention_health = retention_recovery_health_snapshot();
    let expired_backlog_count = if state.config.retention_enabled {
        retention_health
            .expired_backlog_count
            .map(|value| value as u64)
    } else {
        Some(0)
    };
    if !state.raw_capture_circuit.update_inventory_if_current(
        inventory_generation,
        accounting_generation,
        state_name,
        next_raw_bytes,
        None,
        expired_backlog_count,
        Some(spool_bytes),
    ) {
        set_system_raw_metrics_health_override(state, Some("preparing")).await;
        return Ok(0);
    }
    persist_raw_capture_circuit(state).await?;
    set_system_raw_metrics_health_override(state, None).await;
    let circuit_snapshot = state.raw_capture_circuit.snapshot();
    debug!(
        metrics_source = "inventory",
        inventory_state = state_name,
        inventory_cursor = next_cursor,
        link_inventory_cursor = next_link_cursor,
        legacy_row_count = rows.len(),
        link_row_count = link_rows.len(),
        discovered_path_count = deltas.0,
        circuit_state = circuit_snapshot.state,
        circuit_reason = ?circuit_snapshot.reason,
        raw_bytes = circuit_snapshot.raw_bytes,
        available_bytes = ?circuit_snapshot.available_bytes,
        reserved_bytes = circuit_snapshot.reserved_bytes,
        expired_backlog_count = ?circuit_snapshot.expired_backlog_count,
        backlog_non_growing = ?circuit_snapshot.backlog_non_growing,
        raw_close_bytes = RAW_CAPTURE_CLOSE_BYTES,
        raw_resume_bytes = RAW_CAPTURE_RESUME_BYTES,
        available_close_bytes = RAW_CAPTURE_CLOSE_AVAILABLE_BYTES,
        available_resume_bytes = RAW_CAPTURE_RESUME_AVAILABLE_BYTES,
        "system raw metrics inventory batch completed"
    );
    Ok(rows.len().saturating_add(link_rows.len()) as u64)
}

pub(crate) async fn set_system_raw_metrics_health_override(
    state: &AppState,
    override_state: Option<&str>,
) {
    let override_state = override_state.map(str::to_string);
    let mut cache = state.system_status_cache.lock().await;
    if cache.raw_metrics_health_override == override_state {
        return;
    }
    cache.raw_metrics_health_override = override_state.clone();
    if let Some(latest) = cache.latest.as_mut() {
        let display_state = override_state
            .as_deref()
            .unwrap_or(&latest.raw_metrics_inventory_state);
        let display_state = normalize_raw_metrics_state(display_state);
        latest.response.raw_metrics_health.state = display_state.clone();
        latest.response.raw_metrics_health.physical_coverage =
            raw_metrics_physical_coverage(&display_state, &latest.raw_metrics_inventory_state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemRawPayloadInventoryResetOutcome {
    pub(crate) removed_path_count: usize,
    pub(crate) complete: bool,
}

pub(crate) async fn mark_system_raw_payload_metrics_inventory_reset_pending(
    pool: &Pool<Sqlite>,
    max_paths: usize,
    recovery_pending: bool,
) -> Result<SystemRawPayloadInventoryResetOutcome> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE system_raw_payload_metrics
        SET inventory_state = 'resetting',
            inventory_cursor = 0,
            link_inventory_cursor = 0,
            inventory_recheck_cursor = '',
            inventory_recheck_active = 1,
            raw_count = 0,
            raw_bytes = 0,
            request_raw_count = 0,
            request_raw_bytes = 0,
            response_raw_count = 0,
            response_raw_bytes = 0,
            circuit_state = 'unknown',
            circuit_reason = 'inventory_unready',
            circuit_available_bytes = NULL,
            circuit_expired_backlog_count = NULL,
            circuit_backlog_non_growing = NULL,
            circuit_recovery_pending = CASE WHEN ?1 != 0 OR circuit_state = 'storage_suppressed' OR circuit_recovery_pending != 0 THEN 1 ELSE 0 END,
            circuit_updated_at = datetime('now'),
            updated_at = datetime('now')
        WHERE singleton = 1
        "#,
    )
    .bind(i64::from(recovery_pending))
    .execute(tx.as_mut())
    .await?;
    let removed_path_count = sqlx::query(
        r#"
        DELETE FROM system_raw_payload_inventory_paths
        WHERE raw_path IN (
            SELECT raw_path
            FROM system_raw_payload_inventory_paths
            ORDER BY raw_path ASC
            LIMIT ?1
        )
        "#,
    )
    .bind(max_paths.max(1) as i64)
    .execute(tx.as_mut())
    .await?
    .rows_affected() as usize;
    let complete =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM system_raw_payload_inventory_paths")
            .fetch_one(tx.as_mut())
            .await?
            == 0;
    if complete {
        sqlx::query(
            "UPDATE system_raw_payload_metrics SET inventory_state = 'preparing', updated_at = datetime('now') WHERE singleton = 1",
        )
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;
    Ok(SystemRawPayloadInventoryResetOutcome {
        removed_path_count,
        complete,
    })
}

pub(crate) async fn reset_system_raw_payload_metrics_inventory_batch(
    state: &AppState,
    max_paths: usize,
) -> Result<SystemRawPayloadInventoryResetOutcome> {
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE system_raw_payload_metrics
        SET inventory_state = 'resetting',
            inventory_cursor = 0,
            link_inventory_cursor = 0,
            inventory_recheck_cursor = '',
            inventory_recheck_active = 1,
            raw_count = 0,
            raw_bytes = 0,
            request_raw_count = 0,
            request_raw_bytes = 0,
            response_raw_count = 0,
            response_raw_bytes = 0,
            circuit_state = 'unknown',
            circuit_reason = 'inventory_unready',
            circuit_available_bytes = NULL,
            circuit_expired_backlog_count = NULL,
            circuit_backlog_non_growing = NULL,
            circuit_recovery_pending = CASE WHEN circuit_state = 'storage_suppressed' OR circuit_recovery_pending != 0 THEN 1 ELSE 0 END,
            circuit_updated_at = datetime('now'),
            updated_at = datetime('now')
        WHERE singleton = 1
        "#,
    )
    .execute(tx.as_mut())
    .await?;
    let removed_path_count = sqlx::query(
        r#"
        DELETE FROM system_raw_payload_inventory_paths
        WHERE raw_path IN (
            SELECT raw_path
            FROM system_raw_payload_inventory_paths
            ORDER BY raw_path ASC
            LIMIT ?1
        )
        "#,
    )
    .bind(max_paths.max(1) as i64)
    .execute(tx.as_mut())
    .await?
    .rows_affected() as usize;
    let complete =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM system_raw_payload_inventory_paths")
            .fetch_one(tx.as_mut())
            .await?
            == 0;
    if complete {
        sqlx::query(
            "UPDATE system_raw_payload_metrics SET inventory_state = 'preparing', updated_at = datetime('now') WHERE singleton = 1",
        )
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;
    state.raw_capture_circuit.mark_inventory_preparing();
    set_system_raw_metrics_health_override(state, Some("preparing")).await;
    debug!(
        metrics_source = "inventory",
        removed_path_count,
        complete,
        "system raw metrics inventory reset batch completed after retention"
    );
    Ok(SystemRawPayloadInventoryResetOutcome {
        removed_path_count,
        complete,
    })
}

pub(crate) async fn system_raw_payload_metrics_inventory_reset_pending(
    pool: &Pool<Sqlite>,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT inventory_state FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(pool)
    .await?
        == "resetting")
}

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
            } else {
                let circuit_snapshot = state.raw_capture_circuit.snapshot();
                if circuit_snapshot.inventory_state == "ready"
                    || circuit_snapshot.spool_inventory_overflow
                {
                    crate::proxy::recover_raw_overflow_spools_with_circuit(state.as_ref()).await;
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
        }
    })
}
