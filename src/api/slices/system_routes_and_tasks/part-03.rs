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
    result.map(|_| ())
}

fn collect_system_raw_payload_inventory_paths(
    rows: &[SystemRawPayloadInventoryRow],
    link_rows: &[SystemRawPayloadBlobLinkRow],
    fallback_root: Option<&Path>,
) -> HashMap<String, (i64, bool, bool)> {
    let mut paths = HashMap::<String, (i64, bool, bool)>::new();
    for row in rows {
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
    for row in link_rows {
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
    paths
}

struct SystemRawPayloadInventoryBatchResult {
    state: &'static str,
    next_cursor: i64,
    next_link_cursor: i64,
    deltas: (i64, i64, i64, i64, i64, i64),
}

async fn persist_system_raw_payload_inventory_batch(
    state: &AppState,
    snapshot: &SystemRawPayloadMetricsRow,
    rows: &[SystemRawPayloadInventoryRow],
    link_rows: &[SystemRawPayloadBlobLinkRow],
    paths: HashMap<String, (i64, bool, bool)>,
) -> Result<SystemRawPayloadInventoryBatchResult> {
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
    let next_cursor = rows
        .last()
        .map(|row| row.id)
        .unwrap_or(snapshot.inventory_cursor);
    let next_link_cursor = link_rows
        .last()
        .map(|row| row.id)
        .unwrap_or(snapshot.link_inventory_cursor);
    let state = if rows.len() < SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE as usize
        && link_rows.len() < SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE as usize
    {
        "ready"
    } else {
        "preparing"
    };
    sqlx::query(
        r#"
        UPDATE system_raw_payload_metrics
        SET inventory_state = ?1,
            inventory_cursor = ?2,
            link_inventory_cursor = ?3,
            raw_count = raw_count + ?4,
            raw_bytes = raw_bytes + ?5,
            request_raw_count = request_raw_count + ?6,
            request_raw_bytes = request_raw_bytes + ?7,
            response_raw_count = response_raw_count + ?8,
            response_raw_bytes = response_raw_bytes + ?9,
            updated_at = datetime('now')
        WHERE singleton = 1
        "#,
    )
    .bind(state)
    .bind(next_cursor)
    .bind(next_link_cursor)
    .bind(deltas.0)
    .bind(deltas.1)
    .bind(deltas.2)
    .bind(deltas.3)
    .bind(deltas.4)
    .bind(deltas.5)
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    Ok(SystemRawPayloadInventoryBatchResult {
        state,
        next_cursor,
        next_link_cursor,
        deltas,
    })
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
    let snapshot = sqlx::query_as::<_, SystemRawPayloadMetricsRow>(
        "SELECT inventory_state, inventory_cursor, link_inventory_cursor, raw_count, raw_bytes, request_raw_count, request_raw_bytes, response_raw_count, response_raw_bytes, updated_at FROM system_raw_payload_metrics WHERE singleton = 1",
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

    let paths = collect_system_raw_payload_inventory_paths(
        &rows,
        &link_rows,
        state.config.database_path.parent(),
    );
    let batch =
        persist_system_raw_payload_inventory_batch(state, &snapshot, &rows, &link_rows, paths)
            .await?;
    set_system_raw_metrics_health_override(state, None).await;
    debug!(
        metrics_source = "inventory",
        inventory_state = batch.state,
        inventory_cursor = batch.next_cursor,
        link_inventory_cursor = batch.next_link_cursor,
        legacy_row_count = rows.len(),
        link_row_count = link_rows.len(),
        discovered_path_count = batch.deltas.0,
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
        latest.response.raw_metrics_health.state =
            override_state.unwrap_or_else(|| latest.raw_metrics_inventory_state.clone());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemRawPayloadInventoryResetOutcome {
    pub(crate) removed_path_count: usize,
    pub(crate) complete: bool,
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
            raw_count = 0,
            raw_bytes = 0,
            request_raw_count = 0,
            request_raw_bytes = 0,
            response_raw_count = 0,
            response_raw_bytes = 0,
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
