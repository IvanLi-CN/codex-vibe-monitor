use super::*;
use anyhow::anyhow;
use chrono::LocalResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use tokio::sync::{broadcast, watch};
use tracing::{debug, warn};

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

pub(crate) const SYSTEM_STATUS_CACHE_TTL_SECS: u64 = 10;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemStatusMetric {
    pub(crate) count: u64,
    pub(crate) bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemStatusResponse {
    pub(crate) live_invocations_count: u64,
    pub(crate) success_count: u64,
    pub(crate) non_success_count: u64,
    pub(crate) completed_archive_batches_count: u64,
    pub(crate) archived_bodies: SystemStatusMetric,
    pub(crate) raw_bodies: SystemStatusMetric,
    pub(crate) request_raw_bodies: SystemStatusMetric,
    pub(crate) response_raw_bodies: SystemStatusMetric,
    pub(crate) database_bytes: u64,
    pub(crate) other_files_bytes: u64,
    pub(crate) refreshed_at: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemTaskRunResponse {
    pub(crate) id: i64,
    pub(crate) task_kind: String,
    pub(crate) trigger_kind: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
    #[serde(serialize_with = "serialize_local_or_utc_to_utc_iso")]
    pub(crate) started_at: String,
    #[serde(
        serialize_with = "serialize_opt_local_or_utc_to_utc_iso",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemTaskRunsListResponse {
    pub(crate) items: Vec<SystemTaskRunResponse>,
    pub(crate) total: u64,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemTaskRunsQuery {
    pub(crate) task_kind: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) started_at_from: Option<String>,
    pub(crate) started_at_to: Option<String>,
    pub(crate) limit: Option<u32>,
    pub(crate) page: Option<u32>,
    pub(crate) page_size: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct SystemTaskRunHandle {
    pub(crate) id: i64,
    pub(crate) task_kind: SystemTaskKind,
    pub(crate) trigger_kind: String,
    pub(crate) started_at: Instant,
}

#[derive(Debug, FromRow)]
pub(crate) struct SystemTaskRunRow {
    id: i64,
    task_kind: String,
    trigger_kind: String,
    status: String,
    summary: Option<String>,
    detail: Option<String>,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: Option<i64>,
}

#[derive(Debug, Default, FromRow)]
pub(crate) struct SystemInvocationStatusAggRow {
    live_invocations_count: Option<i64>,
    success_count: Option<i64>,
    non_success_count: Option<i64>,
}

#[derive(Debug, Default, FromRow)]
pub(crate) struct SystemArchiveAggRow {
    completed_archive_batches_count: Option<i64>,
    archived_count: Option<i64>,
}

#[derive(Debug, Default, FromRow)]
pub(crate) struct SystemRawBodyPathRow {
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
}

impl From<SystemTaskRunRow> for SystemTaskRunResponse {
    fn from(value: SystemTaskRunRow) -> Self {
        Self {
            id: value.id,
            task_kind: value.task_kind,
            trigger_kind: value.trigger_kind,
            status: value.status,
            summary: value.summary,
            detail: value.detail,
            started_at: value.started_at,
            finished_at: value.finished_at,
            duration_ms: value.duration_ms,
        }
    }
}

pub(crate) fn parse_system_task_run_bound(
    raw: Option<&str>,
    field_name: &str,
) -> Result<Option<String>, ApiError> {
    let Some(raw_value) = normalize_query_text(raw) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(&raw_value)
        .with_context(|| format!("invalid {field_name}: {raw_value}"))
        .map_err(ApiError::bad_request)?
        .with_timezone(&Utc);
    Ok(Some(format_utc_iso(parsed)))
}

pub(crate) fn count_file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

pub(crate) fn add_existing_raw_payload_bytes(
    raw_path: &str,
    fallback_root: Option<&Path>,
    seen_paths: &mut HashSet<PathBuf>,
    metric: &mut SystemStatusMetric,
) {
    let Some(candidate) = resolved_raw_path_read_candidates(raw_path, fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
    else {
        return;
    };
    if !seen_paths.insert(candidate.clone()) {
        return;
    }
    metric.count = metric.count.saturating_add(1);
    metric.bytes = metric.bytes.saturating_add(count_file_size(&candidate));
}

pub(crate) fn collect_existing_raw_payload_metrics(
    rows: &[SystemRawBodyPathRow],
    fallback_root: Option<&Path>,
) -> (SystemStatusMetric, SystemStatusMetric, SystemStatusMetric) {
    let mut total_seen_paths = HashSet::new();
    let mut request_seen_paths = HashSet::new();
    let mut response_seen_paths = HashSet::new();
    let mut total = SystemStatusMetric::default();
    let mut request = SystemStatusMetric::default();
    let mut response = SystemStatusMetric::default();

    for row in rows {
        if let Some(raw_path) = row.request_raw_path.as_deref() {
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut request_seen_paths,
                &mut request,
            );
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut total_seen_paths,
                &mut total,
            );
        }
        if let Some(raw_path) = row.response_raw_path.as_deref() {
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut response_seen_paths,
                &mut response,
            );
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut total_seen_paths,
                &mut total,
            );
        }
    }

    (total, request, response)
}

pub(crate) fn count_database_bytes(db_path: &Path) -> u64 {
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    count_file_size(db_path)
        .saturating_add(count_file_size(&wal_path))
        .saturating_add(count_file_size(&shm_path))
}

pub(crate) fn sum_directory_bytes(root: &Path) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(child),
                Ok(kind) if kind.is_file() => {
                    total =
                        total.saturating_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0));
                }
                _ => {}
            }
        }
    }
    total
}

pub(crate) fn sum_path_bytes(path: &Path) -> u64 {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(metadata) if metadata.is_dir() => sum_directory_bytes(path),
        _ => 0,
    }
}

pub(crate) fn compute_other_files_bytes(
    config: &AppConfig,
    archive_dir: &Path,
    raw_dir: &Path,
) -> u64 {
    let db_path = &config.database_path;
    let db_wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let db_shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    let mut seen = HashSet::new();

    // Keep "other files" scoped to runtime-owned storage that does not already
    // have a dedicated metric on the system status page.
    [config.xray_runtime_dir.clone()]
        .into_iter()
        .filter(|path| !path.as_os_str().is_empty())
        .filter(|path| seen.insert(path.clone()))
        .filter(|path| {
            let candidate = path.as_path();
            candidate != db_path
                && candidate != db_wal_path.as_path()
                && candidate != db_shm_path.as_path()
                && candidate != archive_dir
                && candidate != raw_dir
        })
        .map(|path| sum_path_bytes(&path))
        .sum()
}

pub(crate) async fn load_system_status_uncached(state: &AppState) -> Result<SystemStatusResponse> {
    let invocation_status = sqlx::query_as::<_, SystemInvocationStatusAggRow>(
        r#"
        SELECT
            COUNT(*) AS live_invocations_count,
            COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) = 'success' THEN 1 ELSE 0 END), 0) AS success_count,
            COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) != 'success' THEN 1 ELSE 0 END), 0) AS non_success_count
        FROM codex_invocations
        "#,
    )
    .fetch_one(&state.pool)
    .await?;

    let archived = sqlx::query_as::<_, SystemArchiveAggRow>(
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
    .await?;

    let raw_rows = sqlx::query_as::<_, SystemRawBodyPathRow>(
        r#"
        SELECT
            request_raw_path,
            response_raw_path
        FROM codex_invocations
        WHERE request_raw_path IS NOT NULL
           OR response_raw_path IS NOT NULL
        "#,
    )
    .fetch_all(&state.pool)
    .await?;

    let archive_dir = resolved_archive_dir(&state.config);
    let raw_dir = state.config.resolved_proxy_raw_dir();
    let raw_path_fallback_root = state.config.database_path.parent();
    let (raw_bodies, request_raw_bodies, response_raw_bodies) =
        collect_existing_raw_payload_metrics(&raw_rows, raw_path_fallback_root);
    let archived_paths = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    let mut seen_paths = std::collections::HashSet::new();
    let archive_bytes = archived_paths
        .into_iter()
        .filter(|path| seen_paths.insert(path.clone()))
        .map(PathBuf::from)
        .map(|path| count_file_size(&path))
        .sum();
    let database_bytes = count_database_bytes(&state.config.database_path);
    let other_files_bytes = compute_other_files_bytes(&state.config, &archive_dir, &raw_dir);

    Ok(SystemStatusResponse {
        live_invocations_count: invocation_status.live_invocations_count.unwrap_or(0).max(0) as u64,
        success_count: invocation_status.success_count.unwrap_or(0).max(0) as u64,
        non_success_count: invocation_status.non_success_count.unwrap_or(0).max(0) as u64,
        completed_archive_batches_count: archived
            .completed_archive_batches_count
            .unwrap_or(0)
            .max(0) as u64,
        archived_bodies: SystemStatusMetric {
            count: archived.archived_count.unwrap_or(0).max(0) as u64,
            bytes: archive_bytes,
        },
        raw_bodies,
        request_raw_bodies,
        response_raw_bodies,
        database_bytes,
        other_files_bytes,
        refreshed_at: format_utc_iso(Utc::now()),
    })
}

pub(crate) async fn load_system_status_cached(state: &AppState) -> Result<SystemStatusResponse> {
    {
        let cache = state.system_status_cache.lock().await;
        if let Some(entry) = cache.latest.as_ref()
            && entry.cached_at.elapsed() < Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS)
        {
            return Ok(entry.response.clone());
        }
    }

    let response = load_system_status_uncached(state).await?;
    let mut cache = state.system_status_cache.lock().await;
    cache.latest = Some(SystemStatusCacheEntry {
        cached_at: Instant::now(),
        response: response.clone(),
    });
    Ok(response)
}

pub(crate) async fn invalidate_system_status_cache(state: &AppState) {
    let mut cache = state.system_status_cache.lock().await;
    cache.latest = None;
}

pub(crate) async fn begin_system_task_run(
    pool: &Pool<Sqlite>,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    let started_at = format_utc_iso(Utc::now());
    let trigger_kind = trigger_kind.into();
    let id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO system_task_runs (
            task_kind,
            trigger_kind,
            status,
            summary,
            started_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        RETURNING id
        "#,
    )
    .bind(task_kind.as_str())
    .bind(&trigger_kind)
    .bind(SystemTaskStatus::Running.as_str())
    .bind(summary)
    .bind(&started_at)
    .fetch_one(pool)
    .await?;

    Ok(SystemTaskRunHandle {
        id,
        task_kind,
        trigger_kind,
        started_at: Instant::now(),
    })
}

pub(crate) async fn finish_system_task_run(
    pool: &Pool<Sqlite>,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) {
    let finished_at = format_utc_iso(Utc::now());
    let duration_ms = handle
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    if let Err(err) = sqlx::query(
        r#"
        UPDATE system_task_runs
        SET status = ?1,
            summary = COALESCE(?2, summary),
            detail = ?3,
            finished_at = ?4,
            duration_ms = ?5
        WHERE id = ?6
        "#,
    )
    .bind(status.as_str())
    .bind(summary)
    .bind(detail)
    .bind(&finished_at)
    .bind(duration_ms)
    .bind(handle.id)
    .execute(pool)
    .await
    {
        warn!(
            task_kind = handle.task_kind.as_str(),
            trigger_kind = %handle.trigger_kind,
            error = %err,
            "failed to finalize system task run"
        );
    }
}

pub(crate) async fn finish_system_task_run_batched(
    state: &AppState,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) {
    let finished_at = format_utc_iso(Utc::now());
    let duration_ms = handle
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    if state
        .sqlite_batch_writer
        .enqueue(SqliteBatchWrite::SystemTaskFinish(
            BatchedSystemTaskFinish {
                run_id: handle.id,
                task_kind: handle.task_kind,
                trigger_kind: handle.trigger_kind.clone(),
                status,
                summary: summary.clone(),
                detail: detail.clone(),
                finished_at,
                duration_ms,
            },
        ))
    {
        return;
    }

    finish_system_task_run(&state.pool, handle, status, summary, detail).await;
}

pub(crate) async fn fetch_system_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SystemStatusResponse>, ApiError> {
    Ok(Json(load_system_status_cached(state.as_ref()).await?))
}

pub(crate) async fn list_system_task_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SystemTaskRunsQuery>,
) -> Result<Json<SystemTaskRunsListResponse>, ApiError> {
    let started_at_from =
        parse_system_task_run_bound(query.started_at_from.as_deref(), "startedAtFrom")?;
    let started_at_to = parse_system_task_run_bound(query.started_at_to.as_deref(), "startedAtTo")?;
    let page_size = query
        .page_size
        .unwrap_or(query.limit.unwrap_or(20))
        .clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let limit = i64::from(page_size);
    let offset = i64::from(page.saturating_sub(1)) * limit;
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, task_kind, trigger_kind, status, summary, detail, started_at, finished_at, duration_ms FROM system_task_runs WHERE 1 = 1",
    );
    if let Some(task_kind) = query
        .task_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder.push(" AND task_kind = ").push_bind(task_kind);
    }
    if let Some(status) = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder.push(" AND status = ").push_bind(status);
    }
    if let Some(started_at_from) = started_at_from.as_deref() {
        builder
            .push(" AND datetime(started_at) >= datetime(")
            .push_bind(started_at_from)
            .push(")");
    }
    if let Some(started_at_to) = started_at_to.as_deref() {
        builder
            .push(" AND datetime(started_at) <= datetime(")
            .push_bind(started_at_to)
            .push(")");
    }
    builder
        .push(" ORDER BY started_at DESC, id DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows = builder
        .build_query_as::<SystemTaskRunRow>()
        .fetch_all(&state.pool)
        .await?;

    let mut count_builder =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) as total FROM system_task_runs WHERE 1 = 1");
    if let Some(task_kind) = query
        .task_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        count_builder.push(" AND task_kind = ").push_bind(task_kind);
    }
    if let Some(status) = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        count_builder.push(" AND status = ").push_bind(status);
    }
    if let Some(started_at_from) = started_at_from.as_deref() {
        count_builder
            .push(" AND datetime(started_at) >= datetime(")
            .push_bind(started_at_from)
            .push(")");
    }
    if let Some(started_at_to) = started_at_to.as_deref() {
        count_builder
            .push(" AND datetime(started_at) <= datetime(")
            .push_bind(started_at_to)
            .push(")");
    }
    let total = count_builder
        .build_query_scalar::<i64>()
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(SystemTaskRunsListResponse {
        items: rows.into_iter().map(Into::into).collect(),
        total: total.max(0) as u64,
        page,
        page_size,
    }))
}

pub(crate) fn summarize_retention_run_for_system_task(
    summary: &RetentionRunSummary,
) -> (String, String) {
    let brief = format!(
        "compressed={} archived_invocations={} pruned_details={} orphan_raw_removed={}",
        summary.raw_files_compressed,
        summary.invocation_rows_archived,
        summary.invocation_details_pruned,
        summary.orphan_raw_files_removed
    );
    let detail = format!(
        "dry_run={} raw_candidates={} raw_compressed={} raw_bytes_before={} raw_bytes_after={} details_pruned={} invocation_rows_archived={} forward_proxy_attempt_rows_archived={} pool_attempt_rows_archived={} quota_rows_archived={} archive_batches_touched={} archive_batches_deleted={} raw_files_removed={} orphan_raw_files_removed={}",
        summary.dry_run,
        summary.raw_files_compression_candidates,
        summary.raw_files_compressed,
        summary.raw_bytes_before,
        summary.raw_bytes_after,
        summary.invocation_details_pruned,
        summary.invocation_rows_archived,
        summary.forward_proxy_attempt_rows_archived,
        summary.pool_upstream_request_attempt_rows_archived,
        summary.quota_snapshot_rows_archived,
        summary.archive_batches_touched,
        summary.archive_batches_deleted,
        summary.raw_files_removed,
        summary.orphan_raw_files_removed
    );
    (brief, detail)
}
