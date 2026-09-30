use crate::{AppConfig, OnceCell, Result, Utc, anyhow, format_utc_iso_millis};
use chrono::{Datelike, Duration as ChronoDuration, Timelike};
use serde::Serialize;
use sqlx::{
    FromRow, Pool, Sqlite,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{
    collections::HashSet,
    path::PathBuf,
    str::FromStr,
    sync::atomic::{AtomicI64, Ordering},
    sync::{Mutex, OnceLock},
    time::Duration,
};

const MIN_INTERVAL_SECS: i64 = 60;
const MAX_TASK_ERROR_DETAIL_CHARS: usize = 4_000;
const TASK_RUN_RETENTION_DAYS: i64 = 90;
const TASK_ERROR_RETENTION_DAYS: i64 = 30;
const TASK_HISTORY_CLEANUP_INTERVAL_MS: i64 = 5 * 60 * 1_000;
static LAST_TASK_HISTORY_CLEANUP_MS: AtomicI64 = AtomicI64::new(0);
static ACTIVE_TASK_EXECUTIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub(crate) struct TaskExecutionLease {
    task_key: String,
}

impl Drop for TaskExecutionLease {
    fn drop(&mut self) {
        if let Some(active) = ACTIVE_TASK_EXECUTIONS.get()
            && let Ok(mut active) = active.lock()
        {
            active.remove(&self.task_key);
        }
    }
}

pub(crate) fn try_acquire_task_execution(task_key: &str) -> Option<TaskExecutionLease> {
    let active = ACTIVE_TASK_EXECUTIONS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut active = active.lock().ok()?;
    let canonical_task_key = task_key
        .strip_prefix("startup_backfill.")
        .map(|_| "startup_backfill")
        .unwrap_or(task_key);
    if !active.insert(canonical_task_key.to_string()) {
        return None;
    }
    Some(TaskExecutionLease {
        task_key: canonical_task_key.to_string(),
    })
}

#[derive(Debug, Clone)]
pub(crate) struct MaintenanceStore {
    pub(crate) pool: Pool<Sqlite>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTask {
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) trigger_mode: String,
    pub(crate) enabled: bool,
    pub(crate) interval_secs: Option<i64>,
    pub(crate) cron_expr: Option<String>,
    pub(crate) next_trigger_at: Option<String>,
    pub(crate) is_manual: bool,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskProgress {
    pub(crate) total: Option<i64>,
    pub(crate) completed: Option<i64>,
    pub(crate) phase: Option<String>,
    pub(crate) checkpoint: Option<String>,
    pub(crate) eta_seconds: Option<i64>,
    pub(crate) updated_at: Option<String>,
    pub(crate) freshness: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskRun {
    pub(crate) id: i64,
    pub(crate) trigger_kind: String,
    pub(crate) started_at: String,
    pub(crate) finished_at: Option<String>,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) status: String,
    pub(crate) summary: Option<String>,
    pub(crate) processed_count: Option<i64>,
    pub(crate) updated_count: Option<i64>,
    pub(crate) error_detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTaskDetail {
    pub(crate) task: ManagedTask,
    pub(crate) progress: Option<TaskProgress>,
    pub(crate) recent_runs: Vec<TaskRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) performance: Option<ManagedTaskPerformance>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTaskPerformance {
    pub(crate) run_count: u64,
    pub(crate) success_count: u64,
    pub(crate) failure_count: u64,
    pub(crate) average_duration_ms: Option<f64>,
    pub(crate) latest_duration_ms: Option<f64>,
    pub(crate) observed_at: Option<String>,
}

pub(crate) const MANAGED_TASKS: &[(&str, &str, &str, &str, bool)] = &[
    (
        "retention_archive",
        "数据保留与归档",
        "按保留策略归档并清理历史数据",
        "interval",
        false,
    ),
    (
        "upstream_account_maintenance",
        "上游账号维护",
        "同步账号状态、配额与路由健康信息",
        "interval",
        false,
    ),
    (
        "forward_proxy_subscription_refresh",
        "正向代理订阅刷新",
        "刷新代理订阅并更新代理节点状态",
        "event",
        false,
    ),
    (
        "pool_orphan_recovery",
        "连接池孤儿记录恢复",
        "恢复超时或中断的连接池记录",
        "interval",
        false,
    ),
    (
        "startup_hourly_rollup_bootstrap",
        "启动时小时汇总补齐",
        "补齐启动阶段缺失的小时汇总数据",
        "startup",
        false,
    ),
    (
        "system_status_snapshot",
        "系统状态快照",
        "更新系统状态展示快照",
        "interval",
        false,
    ),
    (
        "invocation_timeline_snapshot",
        "调用时间线快照",
        "生成调用时间线展示所需的异步快照",
        "interval",
        false,
    ),
    (
        "summary_snapshot",
        "汇总快照",
        "更新统计汇总展示快照",
        "event",
        false,
    ),
    (
        "summary_coverage_recovery",
        "汇总覆盖恢复",
        "修复统计汇总的覆盖缺口",
        "interval",
        false,
    ),
    (
        "dashboard_runtime_projection_reconcile",
        "仪表盘运行投影校对",
        "校对仪表盘运行状态投影",
        "event",
        false,
    ),
    (
        "long_term_projection",
        "长期统计投影",
        "更新长期统计投影",
        "interval",
        false,
    ),
    (
        "timeseries_minute_projection",
        "分钟时序投影",
        "更新分钟级时序投影",
        "interval",
        false,
    ),
    (
        "raw_payload_metrics_inventory",
        "原始载荷指标盘点",
        "盘点原始请求与响应载荷的指标",
        "interval",
        false,
    ),
    (
        "prompt_cache_materialization",
        "Prompt 缓存物化",
        "将 Prompt 缓存会话信息物化到展示投影",
        "event",
        false,
    ),
    (
        "startup_backfill",
        "启动回填",
        "补齐历史字段并维护回填进度",
        "startup",
        false,
    ),
    (
        "raw_compression",
        "原始载荷压缩",
        "压缩冷数据原始载荷",
        "manual",
        true,
    ),
    (
        "archive_upstream_activity_manifest",
        "上游活动归档清单",
        "生成上游活动归档清单",
        "manual",
        true,
    ),
    (
        "materialize_historical_rollups",
        "历史汇总物化",
        "物化历史归档批次的统计汇总",
        "manual",
        true,
    ),
    (
        "verify_archive_storage",
        "归档存储校验",
        "校验归档文件、清单与记录一致性",
        "manual",
        true,
    ),
    (
        "prune_archive_batches",
        "归档批次清理",
        "清理符合安全条件的归档批次",
        "manual",
        true,
    ),
    (
        "prune_legacy_archive_batches",
        "旧归档批次清理",
        "清理符合条件的旧版归档批次",
        "manual",
        true,
    ),
];

pub(crate) const STARTUP_BACKFILL_TASKS: &[&str] = &[
    "proxy_usage",
    "prompt_cache_key",
    "prompt_cache_conversations_materialization",
    "requested_service_tier",
    "invocation_service_tier",
    "proxy_cost",
    "reasoning_effort",
    "failure_classification",
    "pool_attempt_public_id_live",
    "pool_attempt_public_id_archives",
    "upstream_activity_live",
    "upstream_activity_archives",
    "pool_upstream_node_health_archives",
    "account_activity_v2_coverage",
    "legacy_detail_mirrors",
    "historical_rollups",
];

fn startup_backfill_task_metadata(key: &str) -> (&'static str, &'static str) {
    match key {
        "proxy_usage" => ("代理用量回填", "回填代理用量字段并记录处理进度"),
        "prompt_cache_key" => ("Prompt 缓存键回填", "回填 Prompt 缓存键并建立关联"),
        "prompt_cache_conversations_materialization" => {
            ("Prompt 缓存会话物化", "物化 Prompt 缓存会话信息")
        }
        "requested_service_tier" => ("请求服务等级回填", "回填请求使用的服务等级"),
        "invocation_service_tier" => ("调用服务等级回填", "回填调用最终使用的服务等级"),
        "proxy_cost" => ("代理成本回填", "根据已记录的调用数据回填代理成本"),
        "reasoning_effort" => ("推理强度回填", "回填调用请求中的推理强度"),
        "failure_classification" => ("失败分类回填", "补齐失败类型与可处理性分类"),
        "pool_attempt_public_id_live" => ("在线连接池尝试 ID 回填", "回填在线连接池尝试的公共 ID"),
        "pool_attempt_public_id_archives" => {
            ("归档连接池尝试 ID 回填", "回填归档连接池尝试的公共 ID")
        }
        "upstream_activity_live" => ("在线上游活动回填", "回填在线上游活动记录"),
        "upstream_activity_archives" => ("归档上游活动回填", "回填归档上游活动记录"),
        "pool_upstream_node_health_archives" => {
            ("上游节点健康归档回填", "回填连接池上游节点的历史健康状态")
        }
        "account_activity_v2_coverage" => ("账号活动 v2 覆盖回填", "补齐账号活动 v2 的覆盖范围"),
        "legacy_detail_mirrors" => ("旧详情镜像回填", "维护旧详情字段的兼容镜像"),
        "historical_rollups" => ("历史汇总回填", "回填历史归档批次的统计汇总"),
        _ => ("启动回填子任务", "执行启动回填的一项历史字段补齐工作"),
    }
}

pub(crate) async fn open(config: &AppConfig) -> Result<MaintenanceStore> {
    let database_path = config.maintenance_database_path();
    if let Some(parent) = database_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let url = format!("sqlite://{}", database_path.to_string_lossy());
    let options = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(2));
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(options)
        .await?;
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;
    ensure_schema(&pool).await?;
    seed_tasks(&pool).await?;
    Ok(MaintenanceStore { pool })
}

fn validate_cron_expr(expr: Option<&str>) -> Result<()> {
    let Some(expr) = expr.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 || fields.iter().any(|field| field.is_empty()) {
        return Err(anyhow!(
            "cron expression must contain exactly five UTC fields"
        ));
    }
    let ranges = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];
    for (field, (minimum, maximum)) in fields.iter().zip(ranges) {
        validate_cron_field(field, minimum, maximum)?;
    }
    Ok(())
}

fn validate_cron_field(field: &str, minimum: u32, maximum: u32) -> Result<()> {
    if field.is_empty() {
        return Err(anyhow!("cron expression contains an empty field"));
    }
    for part in field.split(',') {
        if part.is_empty() {
            return Err(anyhow!("cron expression contains an empty list item"));
        }
        let (base, step) = match part.split_once('/') {
            Some((base, step)) => {
                if step.is_empty() || step.contains('/') {
                    return Err(anyhow!("cron step is invalid"));
                }
                let step = step
                    .parse::<u32>()
                    .map_err(|_| anyhow!("cron step is invalid"))?;
                if step == 0 || step > maximum.saturating_sub(minimum) + 1 {
                    return Err(anyhow!("cron step is out of range"));
                }
                (base, step)
            }
            None => (part, 1),
        };
        if base == "*" {
            continue;
        }
        if let Some((start, end)) = base.split_once('-') {
            let start = start
                .parse::<u32>()
                .map_err(|_| anyhow!("cron range is invalid"))?;
            let end = end
                .parse::<u32>()
                .map_err(|_| anyhow!("cron range is invalid"))?;
            if start < minimum || end > maximum || start > end {
                return Err(anyhow!("cron range is out of range"));
            }
            continue;
        }
        let value = base
            .parse::<u32>()
            .map_err(|_| anyhow!("cron value is invalid"))?;
        if value < minimum || value > maximum {
            return Err(anyhow!("cron value is out of range"));
        }
        if step != 1 {
            return Err(anyhow!("cron step requires a wildcard or range"));
        }
    }
    Ok(())
}

fn cron_field_matches(field: &str, value: u32, minimum: u32, maximum: u32) -> bool {
    field.split(',').any(|part| {
        let (base, step) = part
            .split_once('/')
            .map_or((part, 1), |(base, step)| (base, step.parse().unwrap_or(0)));
        if step == 0 {
            return false;
        }
        if base == "*" {
            return (value - minimum) % step == 0;
        }
        if let Some((start, end)) = base.split_once('-') {
            let Ok(start) = start.parse::<u32>() else {
                return false;
            };
            let Ok(end) = end.parse::<u32>() else {
                return false;
            };
            return start >= minimum
                && end <= maximum
                && start <= end
                && value >= start
                && value <= end
                && (value - start) % step == 0;
        }
        base.parse::<u32>()
            .is_ok_and(|exact| exact == value && exact >= minimum && exact <= maximum)
    })
}

fn cron_field_is_unrestricted(field: &str) -> bool {
    field == "*" || field.starts_with("*/")
}

pub(crate) fn sanitize_task_detail(value: &str) -> String {
    let mut sanitized = value.replace(['\r', '\n'], " ");
    for marker in [
        "Authorization:",
        "authorization=",
        "api_key=",
        "access_token=",
        "token=",
    ] {
        while let Some(start) = sanitized
            .to_ascii_lowercase()
            .find(&marker.to_ascii_lowercase())
        {
            let value_start = start + marker.len();
            let value_slice = &sanitized[value_start..];
            let secret_start = value_start + value_slice.len() - value_slice.trim_start().len();
            let value_end = sanitized[secret_start..]
                .find(|character: char| {
                    character.is_whitespace() || character == ',' || character == ';'
                })
                .map_or(sanitized.len(), |offset| secret_start + offset);
            sanitized.replace_range(start..value_end, "[REDACTED]");
        }
    }
    sanitized
        .chars()
        .take(MAX_TASK_ERROR_DETAIL_CHARS)
        .collect()
}

fn next_trigger_at(interval_secs: Option<i64>, cron_expr: Option<&str>) -> Option<String> {
    let now = Utc::now();
    if let Some(expr) = cron_expr.map(str::trim).filter(|value| !value.is_empty()) {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return None;
        }
        let base = now
            - ChronoDuration::seconds(i64::from(now.second()))
            - ChronoDuration::nanoseconds(i64::from(now.nanosecond()));
        let dom_unrestricted = cron_field_is_unrestricted(fields[2]);
        let dow_unrestricted = cron_field_is_unrestricted(fields[4]);
        for offset in 1..=(366 * 24 * 60) {
            let candidate = base + ChronoDuration::minutes(offset);
            let dom_match = cron_field_matches(fields[2], candidate.day(), 1, 31);
            let dow_match =
                cron_field_matches(fields[4], candidate.weekday().num_days_from_sunday(), 0, 6);
            let day_match = if dom_unrestricted || dow_unrestricted {
                dom_match && dow_match
            } else {
                dom_match || dow_match
            };
            if cron_field_matches(fields[0], candidate.minute(), 0, 59)
                && cron_field_matches(fields[1], candidate.hour(), 0, 23)
                && cron_field_matches(fields[3], candidate.month(), 1, 12)
                && day_match
            {
                return Some(format_utc_iso_millis(candidate));
            }
        }
        return None;
    }
    interval_secs
        .filter(|seconds| *seconds >= MIN_INTERVAL_SECS)
        .map(|seconds| format_utc_iso_millis(now + ChronoDuration::seconds(seconds)))
}

async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    for statement in r#"
        CREATE TABLE IF NOT EXISTS managed_tasks (
          task_key TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL,
          trigger_mode TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
          interval_secs INTEGER, cron_expr TEXT, next_trigger_at TEXT,
          is_manual INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS managed_task_progress (
          task_key TEXT PRIMARY KEY REFERENCES managed_tasks(task_key) ON DELETE CASCADE,
          total INTEGER, completed INTEGER, phase TEXT, checkpoint TEXT, eta_seconds INTEGER,
          updated_at TEXT, freshness TEXT NOT NULL DEFAULT 'fresh'
        );
        CREATE TABLE IF NOT EXISTS startup_backfill_progress (
          task_name TEXT PRIMARY KEY,
          cursor_id INTEGER NOT NULL DEFAULT 0,
          next_run_after TEXT,
          zero_update_streak INTEGER NOT NULL DEFAULT 0,
          last_started_at TEXT,
          last_finished_at TEXT,
          last_scanned INTEGER NOT NULL DEFAULT 0,
          last_updated INTEGER NOT NULL DEFAULT 0,
          last_status TEXT NOT NULL DEFAULT 'idle',
          suspension_reason TEXT,
          next_probe_at TEXT,
          wake_generation INTEGER NOT NULL DEFAULT 0,
          enabled INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS managed_task_runs (
          id INTEGER PRIMARY KEY AUTOINCREMENT, legacy_id INTEGER UNIQUE, task_key TEXT NOT NULL,
          trigger_kind TEXT NOT NULL DEFAULT 'unknown', started_at TEXT NOT NULL, finished_at TEXT,
          duration_ms INTEGER, status TEXT NOT NULL, summary TEXT,
          processed_count INTEGER, updated_count INTEGER, error_detail TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_managed_task_runs_task_started
          ON managed_task_runs(task_key, started_at DESC);
    "#
    .split(';')
    .map(str::trim)
    .filter(|statement| !statement.is_empty())
    {
        sqlx::query(statement).execute(pool).await?;
    }
    // Existing maintenance databases may predate legacy_id; keep upgrades additive.
    let has_legacy_id: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('managed_task_runs') WHERE name = 'legacy_id'",
    )
    .fetch_optional(pool)
    .await?;
    if has_legacy_id.is_none() {
        sqlx::query("ALTER TABLE managed_task_runs ADD COLUMN legacy_id INTEGER")
            .execute(pool)
            .await?;
    }
    for (column, definition) in [
        ("trigger_kind", "TEXT NOT NULL DEFAULT 'unknown'"),
        ("summary", "TEXT"),
    ] {
        let present: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT 1 FROM pragma_table_info('managed_task_runs') WHERE name = '{column}'"
        ))
        .fetch_optional(pool)
        .await?;
        if present.is_none() {
            sqlx::query(&format!(
                "ALTER TABLE managed_task_runs ADD COLUMN {column} {definition}"
            ))
            .execute(pool)
            .await?;
        }
    }
    let has_next_trigger_at: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('managed_tasks') WHERE name = 'next_trigger_at'",
    )
    .fetch_optional(pool)
    .await?;
    if has_next_trigger_at.is_none() {
        sqlx::query("ALTER TABLE managed_tasks ADD COLUMN next_trigger_at TEXT")
            .execute(pool)
            .await?;
    }
    // Keep databases written by the old dual-field form deterministic: an explicit cron wins.
    sqlx::query(
        "UPDATE managed_tasks SET interval_secs=NULL WHERE cron_expr IS NOT NULL AND trim(cron_expr) <> '' AND interval_secs IS NOT NULL",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_task_runs_legacy_id ON managed_task_runs(legacy_id) WHERE legacy_id IS NOT NULL")
        .execute(pool)
        .await?;
    sqlx::query(
        "DELETE FROM managed_task_runs WHERE id IN (SELECT duplicate.id FROM managed_task_runs duplicate JOIN managed_task_runs kept ON kept.task_key = duplicate.task_key AND kept.status IN ('running','requested') AND duplicate.status IN ('running','requested') AND kept.id < duplicate.id)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_task_active_run ON managed_task_runs(task_key) WHERE status IN ('running','requested')",
    )
    .execute(pool)
    .await?;
    let now = Utc::now();
    let error_cutoff = format_utc_iso_millis(now - ChronoDuration::days(TASK_ERROR_RETENTION_DAYS));
    let run_cutoff = format_utc_iso_millis(now - ChronoDuration::days(TASK_RUN_RETENTION_DAYS));
    sqlx::query("UPDATE managed_task_runs SET error_detail=NULL WHERE started_at < ?")
        .bind(error_cutoff)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM managed_task_runs WHERE started_at < ?")
        .bind(run_cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

async fn seed_tasks(pool: &Pool<Sqlite>) -> Result<()> {
    let now = format_utc_iso_millis(Utc::now());
    for (key, title, description, mode, manual) in MANAGED_TASKS {
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,is_manual,updated_at) VALUES (?,?,?,?,?,?) ON CONFLICT(task_key) DO UPDATE SET title=excluded.title, description=excluded.description, trigger_mode=excluded.trigger_mode, is_manual=excluded.is_manual")
            .bind(key).bind(title).bind(description).bind(mode).bind(*manual as i64).bind(&now).execute(pool).await?;
    }
    for key in STARTUP_BACKFILL_TASKS {
        let (title, description) = startup_backfill_task_metadata(key);
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,is_manual,updated_at) VALUES (?,?,?,?,?,?) ON CONFLICT(task_key) DO UPDATE SET title=excluded.title, description=excluded.description, trigger_mode=excluded.trigger_mode, is_manual=excluded.is_manual")
            .bind(format!("startup_backfill.{key}")).bind(title).bind(description).bind("event").bind(0_i64).bind(&now).execute(pool).await?;
    }
    Ok(())
}

static GLOBAL: OnceCell<std::sync::Arc<MaintenanceStore>> = OnceCell::const_new();

pub(crate) fn set_global(store: std::sync::Arc<MaintenanceStore>) {
    let _ = GLOBAL.set(store);
}

pub(crate) fn global() -> Option<&'static std::sync::Arc<MaintenanceStore>> {
    GLOBAL.get()
}

pub(crate) async fn legacy_worker_should_skip(task_key: &str) -> bool {
    let Some(store) = global() else {
        return false;
    };
    let Some((enabled, interval_secs, cron_expr)) =
        sqlx::query_as::<_, (bool, Option<i64>, Option<String>)>(
            "SELECT enabled,interval_secs,cron_expr FROM managed_tasks WHERE task_key=?",
        )
        .bind(task_key)
        .fetch_optional(&store.pool)
        .await
        .ok()
        .flatten()
    else {
        return false;
    };
    !enabled
        || interval_secs.is_some()
        || cron_expr
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

impl MaintenanceStore {
    pub(crate) async fn claim_requested_run(&self) -> Result<Option<(i64, String, String)>> {
        let mut transaction = self.pool.begin().await?;
        let claimed = sqlx::query_as::<_, (i64, String, String)>(
            "UPDATE managed_task_runs
             SET status='running'
             WHERE id = (
                 SELECT id FROM managed_task_runs
                 WHERE status='requested'
                 ORDER BY id
                 LIMIT 1
             )
             AND status='requested'
             RETURNING id,task_key,started_at",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((id, task_key, started_at)) = claimed else {
            transaction.commit().await?;
            return Ok(None);
        };
        transaction.commit().await?;
        Ok(Some((id, task_key, started_at)))
    }

    pub(crate) async fn validate_schedule(
        interval_secs: Option<i64>,
        cron_expr: Option<&str>,
    ) -> Result<()> {
        if interval_secs.is_some()
            && cron_expr
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        {
            return Err(anyhow!("interval and cron schedule are mutually exclusive"));
        }
        Self::validate_interval(interval_secs).await?;
        validate_cron_expr(cron_expr)
    }

    pub(crate) async fn request_run(&self, task_key: &str) -> Result<i64> {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM managed_tasks WHERE task_key=?")
                .bind(task_key)
                .fetch_optional(&self.pool)
                .await?;
        if exists.is_none() {
            return Err(anyhow!("managed task not found"));
        }
        let result = sqlx::query_scalar("INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary,error_detail) VALUES (?,?,?,?,?,?) RETURNING id")
            .bind(task_key)
            .bind("manual")
            .bind(format_utc_iso_millis(Utc::now()))
            .bind("requested")
            .bind("手动运行请求")
            .bind(Option::<String>::None)
            .fetch_one(&self.pool)
            .await;
        result.map_err(|error| {
            if error
                .to_string()
                .contains("UNIQUE constraint failed: managed_task_runs.task_key")
            {
                anyhow!("task already has an active run")
            } else {
                error.into()
            }
        })
    }

    pub(crate) async fn migrate_legacy_state(&self, main_pool: &Pool<Sqlite>) -> Result<()> {
        let legacy_runs = sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, String, Option<String>, Option<i64>)>(
            "SELECT id, task_kind, trigger_kind, status, summary, detail, started_at, finished_at, duration_ms FROM system_task_runs ORDER BY id",
        )
        .fetch_all(main_pool)
        .await?;
        for (
            id,
            task_kind,
            trigger_kind,
            status,
            summary,
            detail,
            started_at,
            finished_at,
            duration_ms,
        ) in legacy_runs
        {
            let task_key = match task_kind.as_str() {
                "hourly_rollup_bootstrap" => "startup_hourly_rollup_bootstrap",
                "startup_backfill" => "startup_backfill",
                other => other,
            };
            let error_detail = detail.as_deref().map(sanitize_task_detail);
            sqlx::query(
                "INSERT OR IGNORE INTO managed_task_runs (legacy_id,task_key,trigger_kind,started_at,finished_at,duration_ms,status,summary,error_detail) VALUES (?,?,?,?,?,?,?,?,?)",
            )
            .bind(id)
            .bind(task_key)
            .bind(trigger_kind)
            .bind(started_at)
            .bind(finished_at)
            .bind(duration_ms)
            .bind(status)
            .bind(summary)
            .bind(error_detail)
            .execute(&self.pool)
            .await?;
        }

        let progress = sqlx::query_as::<_, (String, i64, Option<String>, i64, Option<String>, Option<String>, i64, i64, String, Option<String>, Option<String>, i64, i64)>(
            "SELECT task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled FROM startup_backfill_progress",
        )
        .fetch_all(main_pool)
        .await?;
        for (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status,
            suspension_reason,
            next_probe_at,
            wake_generation,
            enabled,
        ) in progress
        {
            sqlx::query(
                "INSERT OR IGNORE INTO startup_backfill_progress (task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&task_name)
            .bind(cursor_id)
            .bind(&next_run_after)
            .bind(zero_update_streak)
            .bind(&last_started_at)
            .bind(&last_finished_at)
            .bind(last_scanned)
            .bind(last_updated)
            .bind(&last_status)
            .bind(&suspension_reason)
            .bind(&next_probe_at)
            .bind(wake_generation)
            .bind(enabled)
            .execute(&self.pool)
            .await?;
            let task_key = format!("startup_backfill.{task_name}");
            let updated_at = last_finished_at.or(last_started_at).or(next_run_after);
            let freshness = if enabled == 0 { "stale" } else { "fresh" };
            sqlx::query(
                "INSERT OR IGNORE INTO managed_task_progress (task_key,total,completed,phase,checkpoint,updated_at,freshness) VALUES (?,?,?,?,?,?,?)",
            )
            .bind(task_key)
            .bind(Option::<i64>::None)
            .bind(last_updated.max(last_scanned))
            .bind(last_status)
            .bind(cursor_id.to_string())
            .bind(updated_at)
            .bind(freshness)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn enqueue_due_runs(&self) -> Result<u64> {
        let now = Utc::now();
        let now_text = format_utc_iso_millis(now);
        let mut transaction = self.pool.begin().await?;
        let due_tasks = sqlx::query_as::<_, (String, Option<i64>, Option<String>)>(
            "SELECT task_key,interval_secs,cron_expr
             FROM managed_tasks
             WHERE enabled=1 AND is_manual=0 AND next_trigger_at IS NOT NULL
               AND next_trigger_at <= ?
             ORDER BY next_trigger_at, task_key",
        )
        .bind(&now_text)
        .fetch_all(&mut *transaction)
        .await?;
        let mut enqueued = 0_u64;
        for (task_key, interval_secs, cron_expr) in due_tasks {
            let next_trigger = next_trigger_at(interval_secs, cron_expr.as_deref());
            let inserted = sqlx::query(
                "INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary)
                 VALUES (?,?,?,?,?)
                 ON CONFLICT DO NOTHING",
            )
            .bind(&task_key)
            .bind("schedule")
            .bind(&now_text)
            .bind("requested")
            .bind("按计划触发")
            .execute(&mut *transaction)
            .await?;
            enqueued += inserted.rows_affected();
            sqlx::query(
                "UPDATE managed_tasks SET next_trigger_at=?, updated_at=? WHERE task_key=?",
            )
            .bind(next_trigger)
            .bind(&now_text)
            .bind(task_key)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(enqueued)
    }

    pub(crate) async fn cleanup_expired_history_if_due(&self) -> Result<()> {
        let now_ms = Utc::now().timestamp_millis();
        let last_ms = LAST_TASK_HISTORY_CLEANUP_MS.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last_ms) < TASK_HISTORY_CLEANUP_INTERVAL_MS
            || LAST_TASK_HISTORY_CLEANUP_MS
                .compare_exchange(last_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return Ok(());
        }
        let now = Utc::now();
        let error_cutoff =
            format_utc_iso_millis(now - ChronoDuration::days(TASK_ERROR_RETENTION_DAYS));
        let run_cutoff = format_utc_iso_millis(now - ChronoDuration::days(TASK_RUN_RETENTION_DAYS));
        sqlx::query("UPDATE managed_task_runs SET error_detail=NULL WHERE started_at < ?")
            .bind(error_cutoff)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM managed_task_runs WHERE started_at < ?")
            .bind(run_cutoff)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn begin_run(
        &self,
        task_key: &str,
        started_at: &str,
        trigger_kind: &str,
        summary: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query_scalar("INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary) VALUES (?,?,?,?,?) RETURNING id")
            .bind(task_key)
            .bind(trigger_kind)
            .bind(started_at)
            .bind("running")
            .bind(summary)
            .fetch_one(&self.pool)
            .await;
        result.map_err(|error| {
            if error
                .to_string()
                .contains("UNIQUE constraint failed: managed_task_runs.task_key")
            {
                anyhow!("task already has an active run")
            } else {
                error.into()
            }
        })
    }

    pub(crate) async fn finish_run(
        &self,
        id: i64,
        status: &str,
        finished_at: &str,
        duration_ms: i64,
        summary: Option<&str>,
        detail: Option<&str>,
    ) -> Result<()> {
        let sanitized = detail.map(sanitize_task_detail);
        sqlx::query("UPDATE managed_task_runs SET status=?,finished_at=?,duration_ms=?,summary=COALESCE(?,summary),error_detail=? WHERE id=?")
            .bind(status).bind(finished_at).bind(duration_ms).bind(summary).bind(sanitized).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub(crate) async fn list_tasks(&self) -> Result<Vec<ManagedTask>> {
        Ok(sqlx::query_as("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,next_trigger_at,is_manual FROM managed_tasks ORDER BY task_key")
        .fetch_all(&self.pool).await?)
    }

    pub(crate) async fn detail(&self, task_key: &str) -> Result<Option<ManagedTaskDetail>> {
        let task = sqlx::query_as::<_, ManagedTask>("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,next_trigger_at,is_manual FROM managed_tasks WHERE task_key=?")
        .bind(task_key).fetch_optional(&self.pool).await?;
        let Some(task) = task else {
            return Ok(None);
        };
        let progress = sqlx::query_as::<_, TaskProgress>("SELECT total,completed,phase,checkpoint,eta_seconds,updated_at,freshness FROM managed_task_progress WHERE task_key=?")
        .bind(task_key).fetch_optional(&self.pool).await?;
        let recent_runs = sqlx::query_as::<_, TaskRun>("SELECT id,trigger_kind,started_at,finished_at,duration_ms,status,summary,processed_count,updated_count,error_detail FROM managed_task_runs WHERE task_key=? ORDER BY started_at DESC LIMIT 10")
        .bind(task_key).fetch_all(&self.pool).await?;
        Ok(Some(ManagedTaskDetail {
            task,
            progress,
            recent_runs,
            performance: None,
        }))
    }

    pub(crate) async fn set_enabled(&self, task_key: &str, enabled: bool) -> Result<bool> {
        let Some((interval_secs, cron_expr, is_manual)) =
            sqlx::query_as::<_, (Option<i64>, Option<String>, bool)>(
                "SELECT interval_secs,cron_expr,is_manual FROM managed_tasks WHERE task_key=?",
            )
            .bind(task_key)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(false);
        };
        let next_trigger_at = if enabled && !is_manual {
            next_trigger_at(interval_secs, cron_expr.as_deref())
        } else {
            None
        };
        sqlx::query(
            "UPDATE managed_tasks SET enabled=?, next_trigger_at=?, updated_at=? WHERE task_key=?",
        )
        .bind(enabled as i64)
        .bind(next_trigger_at)
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(task_key)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    pub(crate) async fn validate_interval(interval_secs: Option<i64>) -> Result<()> {
        if let Some(value) = interval_secs
            && value < MIN_INTERVAL_SECS
        {
            return Err(anyhow!(
                "interval must be at least {MIN_INTERVAL_SECS} seconds"
            ));
        }
        Ok(())
    }

    pub(crate) async fn set_schedule(
        &self,
        task_key: &str,
        interval_secs: Option<i64>,
        cron_expr: Option<&str>,
    ) -> Result<bool> {
        Self::validate_schedule(interval_secs, cron_expr).await?;
        let Some(enabled) = sqlx::query_scalar::<_, bool>(
            "SELECT enabled FROM managed_tasks WHERE task_key=? AND is_manual=0",
        )
        .bind(task_key)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(false);
        };
        let next_trigger_at = if enabled {
            next_trigger_at(interval_secs, cron_expr)
        } else {
            None
        };
        sqlx::query("UPDATE managed_tasks SET interval_secs=?, cron_expr=?, next_trigger_at=?, updated_at=? WHERE task_key=? AND is_manual=0")
        .bind(interval_secs).bind(cron_expr).bind(next_trigger_at).bind(format_utc_iso_millis(Utc::now())).bind(task_key).execute(&self.pool).await?;
        Ok(true)
    }

    pub(crate) async fn update_control(
        &self,
        task_key: &str,
        enabled: Option<bool>,
        interval_secs: Option<i64>,
        cron_expr: Option<&str>,
        update_schedule: bool,
    ) -> Result<bool> {
        if update_schedule {
            Self::validate_schedule(interval_secs, cron_expr).await?;
        }
        let mut transaction = self.pool.begin().await?;
        let Some((current_enabled, current_interval, current_cron, is_manual)) = sqlx::query_as::<
            _,
            (bool, Option<i64>, Option<String>, bool),
        >(
            "SELECT enabled,interval_secs,cron_expr,is_manual FROM managed_tasks WHERE task_key=?",
        )
        .bind(task_key)
        .fetch_optional(&mut *transaction)
        .await?
        else {
            transaction.commit().await?;
            return Ok(false);
        };
        if update_schedule && is_manual {
            transaction.commit().await?;
            return Err(anyhow!("manual or unknown task cannot be scheduled"));
        }
        let next_enabled = enabled.unwrap_or(current_enabled);
        let next_interval = if update_schedule {
            interval_secs
        } else {
            current_interval
        };
        let next_cron = if update_schedule {
            cron_expr.map(str::to_owned)
        } else {
            current_cron
        };
        let next_trigger_at = if next_enabled && !is_manual {
            next_trigger_at(next_interval, next_cron.as_deref())
        } else {
            None
        };
        sqlx::query(
            "UPDATE managed_tasks SET enabled=?,interval_secs=?,cron_expr=?,next_trigger_at=?,updated_at=? WHERE task_key=?",
        )
        .bind(next_enabled as i64)
        .bind(next_interval)
        .bind(next_cron)
        .bind(next_trigger_at)
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(task_key)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }
}

pub(crate) fn path(config: &AppConfig) -> PathBuf {
    config.maintenance_database_path()
}

#[cfg(test)]
mod tests {
    use chrono::{Timelike, Utc};

    use super::{
        MANAGED_TASKS, STARTUP_BACKFILL_TASKS, next_trigger_at, sanitize_task_detail,
        validate_cron_expr,
    };

    #[test]
    fn managed_task_registry_matches_the_operations_catalog() {
        assert_eq!(MANAGED_TASKS.len(), 21);
        assert_eq!(
            MANAGED_TASKS
                .iter()
                .filter(|(_, _, _, _, is_manual)| *is_manual)
                .count(),
            6
        );
        assert_eq!(STARTUP_BACKFILL_TASKS.len(), 16);
    }

    #[test]
    fn computes_interval_next_trigger_after_the_minimum_safety_window() {
        let before = Utc::now();
        let next = next_trigger_at(Some(60), None).expect("interval should produce a trigger");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next)
            .expect("next trigger should be RFC3339")
            .with_timezone(&Utc);

        assert!(parsed >= before + chrono::Duration::seconds(59));
        assert!(parsed <= before + chrono::Duration::seconds(61));
    }

    #[test]
    fn computes_the_next_utc_cron_minute() {
        let now = Utc::now();
        let next = next_trigger_at(None, Some("*/5 * * * *"))
            .expect("five-minute cron should produce a trigger");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next)
            .expect("next trigger should be RFC3339")
            .with_timezone(&Utc);

        assert!(parsed > now);
        assert_eq!(parsed.minute() % 5, 0);
        assert_eq!(parsed.second(), 0);
        assert_eq!(parsed.nanosecond(), 0);
    }

    #[test]
    fn rejects_cron_expressions_without_five_utc_fields() {
        assert!(validate_cron_expr(Some("*/5 * * *")).is_err());
        assert!(validate_cron_expr(Some("*/5 * * * *")).is_ok());
    }

    #[test]
    fn rejects_out_of_range_and_zero_step_cron_fields() {
        assert!(validate_cron_expr(Some("61 * * * *")).is_err());
        assert!(validate_cron_expr(Some("*/0 * * * *")).is_err());
        assert!(validate_cron_expr(Some("1-0 * * * *")).is_err());
    }

    #[test]
    fn sanitizes_and_bounds_task_error_details() {
        let detail = sanitize_task_detail("Authorization: secret\napi_key=abc, remaining");
        assert!(!detail.contains("secret"));
        assert!(!detail.contains("abc"));
        assert_eq!(sanitize_task_detail(&"x".repeat(5_000)).len(), 4_000);
    }
}
