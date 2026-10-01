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
const TASK_PROGRESS_STALE_AFTER_SECS: i64 = 30;
const TASK_TIMELINE_RETENTION_HOURS: i64 = 48;
const TASK_RUNNING_RECOVERY_STALE_AFTER_SECS: i64 = 5 * 60;
const INITIAL_TASK_DEFAULTS_MARKER: &str = "managed_task_defaults_v1";
const RETENTION_DEFAULT_SCHEDULE_MARKER: &str = "retention_default_schedule_v1";
const DEFAULT_RETENTION_INTERVAL_SECS: i64 = 3_600;
const LEGACY_BACKFILL_ENABLEMENT_MARKER: &str = "managed_task_legacy_enablement_v1";
const TASK_DISABLED_UNTIL_DAYS: i64 = 3650;
const DEFAULT_ENABLED_TASKS: &[&str] = &[
    "retention_archive",
    "upstream_account_maintenance",
    "forward_proxy_subscription_refresh",
    "pool_orphan_recovery",
    "startup_hourly_rollup_bootstrap",
    "system_status_snapshot",
    "invocation_timeline_snapshot",
    "summary_snapshot",
    "summary_coverage_recovery",
    "dashboard_runtime_projection_reconcile",
    "long_term_projection",
    "timeseries_minute_projection",
    "raw_payload_metrics_inventory",
    "prompt_cache_materialization",
];
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
    let canonical_task_key = task_key
        .strip_prefix("startup_backfill.")
        .map(|_| "startup_backfill")
        .unwrap_or(task_key);
    #[cfg(test)]
    {
        Some(TaskExecutionLease {
            task_key: canonical_task_key.to_string(),
        })
    }
    #[cfg(not(test))]
    {
        let active = ACTIVE_TASK_EXECUTIONS.get_or_init(|| Mutex::new(HashSet::new()));
        let mut active = active.lock().ok()?;
        if !active.insert(canonical_task_key.to_string()) {
            return None;
        }
        Some(TaskExecutionLease {
            task_key: canonical_task_key.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MaintenanceStore {
    pub(crate) pool: Pool<Sqlite>,
}

#[derive(Debug, Clone, Default, Serialize, FromRow)]
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
    pub(crate) display_color_light: Option<String>,
    pub(crate) display_color_dark: Option<String>,
    #[serde(skip)]
    pub(crate) schedule_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[sqlx(skip)]
    pub(crate) effective_schedule: Option<ManagedTaskSchedule>,
    #[sqlx(skip)]
    pub(crate) trigger_kinds: Vec<String>,
    #[sqlx(skip)]
    pub(crate) effective_policy: String,
    #[sqlx(skip)]
    pub(crate) policy_source: String,
    #[sqlx(skip)]
    pub(crate) schedule_editable: bool,
    #[sqlx(skip)]
    pub(crate) schedule_capability_reason: Option<String>,
    #[sqlx(skip)]
    pub(crate) execution_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTaskSchedule {
    pub(crate) source: String,
    pub(crate) interval_secs: Option<i64>,
    pub(crate) cron_expr: Option<String>,
    pub(crate) next_trigger_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskProgress {
    pub(crate) total: Option<i64>,
    pub(crate) completed: Option<i64>,
    pub(crate) phase: Option<String>,
    pub(crate) checkpoint: Option<String>,
    pub(crate) eta_seconds: Option<i64>,
    pub(crate) updated_at: Option<String>,
    pub(crate) freshness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_progress_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) wait_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_retry_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stages: Option<Vec<TaskStage>>,
}

#[derive(Debug, Clone, Serialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) core_completion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskStage {
    pub(crate) name: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) elapsed_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) wait_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) coverage: Option<f64>,
}

#[derive(Debug, FromRow)]
struct TaskProgressRow {
    total: Option<i64>,
    completed: Option<i64>,
    phase: Option<String>,
    checkpoint: Option<String>,
    eta_seconds: Option<i64>,
    updated_at: Option<String>,
    freshness: String,
    unit: Option<String>,
    source_scope: Option<String>,
    last_progress_at: Option<String>,
    wait_reason: Option<String>,
    next_retry_at: Option<String>,
    stages: Option<String>,
}

#[derive(Debug, FromRow)]
struct TaskRunRow {
    id: i64,
    trigger_kind: String,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: Option<i64>,
    status: String,
    summary: Option<String>,
    processed_count: Option<i64>,
    updated_count: Option<i64>,
    error_detail: Option<String>,
    completion: Option<String>,
    core_completion: Option<String>,
    details: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueuedTaskRun {
    pub(crate) run_id: i64,
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) trigger_kind: String,
    pub(crate) requested_at: String,
    pub(crate) waiting_ms: i64,
    pub(crate) position: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskAdmissionWait {
    pub(crate) id: String,
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) reason: String,
    pub(crate) started_at: String,
    pub(crate) waiting_ms: i64,
    pub(crate) retry_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineSegment {
    pub(crate) segment_id: String,
    pub(crate) kind: String,
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) started_at: String,
    pub(crate) last_observed_at: String,
    pub(crate) finished_at: Option<String>,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) status: String,
    pub(crate) trigger_kind: Option<String>,
    pub(crate) execution_class: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) retry_at: Option<String>,
    pub(crate) active_child_task_key: Option<String>,
    pub(crate) active_child_title: Option<String>,
    pub(crate) managed_run_id: Option<i64>,
    pub(crate) session_id: String,
    pub(crate) revision: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineCoverage {
    pub(crate) session_id: String,
    pub(crate) started_at: String,
    pub(crate) last_seen_at: String,
    pub(crate) ended_at: Option<String>,
    pub(crate) dropped_events: i64,
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

pub(crate) fn task_title_for_observation(task_key: &str) -> String {
    if let Some((_, title, _, _, _)) = MANAGED_TASKS.iter().find(|(key, ..)| *key == task_key) {
        return (*title).to_string();
    }
    if let Some(child) = task_key.strip_prefix("startup_backfill.") {
        return startup_backfill_task_metadata(child).0.to_string();
    }
    task_key.to_string()
}

pub(crate) fn task_execution_class(task_key: &str) -> Option<&'static str> {
    match task_key {
        "retention_archive"
        | "upstream_account_maintenance"
        | "pool_orphan_recovery"
        | "invocation_timeline_snapshot"
        | "raw_payload_metrics_inventory" => Some("maintenance_retention"),
        "dashboard_runtime_projection_reconcile"
        | "forward_proxy_subscription_refresh"
        | "long_term_projection"
        | "timeseries_minute_projection" => Some("p2_derived"),
        _ => None,
    }
}

pub(crate) fn task_enabled_by_default(task_key: &str, is_manual: bool) -> bool {
    !is_manual && DEFAULT_ENABLED_TASKS.contains(&task_key)
}

const EDITABLE_SCHEDULE_TASKS: &[&str] = &[
    "retention_archive",
    "upstream_account_maintenance",
    "pool_orphan_recovery",
    "system_status_snapshot",
    "invocation_timeline_snapshot",
    "dashboard_runtime_projection_reconcile",
];

fn task_trigger_kinds(task_key: &str, trigger_mode: &str, is_manual: bool) -> Vec<String> {
    if is_manual || trigger_mode == "manual" {
        return vec!["manual".to_string()];
    }
    let kinds: &[&str] = match task_key {
        "retention_archive" | "upstream_account_maintenance" => &["startup", "interval"],
        "forward_proxy_subscription_refresh" => &["startup", "interval"],
        "dashboard_runtime_projection_reconcile" => &["interval", "adaptive"],
        "summary_snapshot" | "prompt_cache_materialization" => &["event", "interval"],
        "summary_coverage_recovery" | "long_term_projection" => &["adaptive", "interval"],
        "timeseries_minute_projection" => &["startup", "interval", "adaptive"],
        "startup_backfill" => &["startup", "event", "adaptive"],
        key if key.starts_with("startup_backfill.") => &["event", "interval"],
        _ if trigger_mode == "startup" => &["startup"],
        _ if trigger_mode == "event" => &["event"],
        _ => &["interval"],
    };
    kinds.iter().map(|kind| (*kind).to_string()).collect()
}

fn task_policy_text(
    task_key: &str,
    interval_secs: Option<i64>,
    cron_expr: Option<&str>,
) -> (&'static str, String) {
    if let Some(cron) = cron_expr.map(str::trim).filter(|value| !value.is_empty()) {
        return ("运维自定义", format!("UTC cron：{cron}"));
    }
    if let Some(interval) = interval_secs {
        return ("运维自定义", format!("固定检查间隔：{interval} 秒"));
    }
    if MANAGED_TASKS
        .iter()
        .any(|(key, .., is_manual)| *key == task_key && *is_manual)
    {
        return ("系统规则", "手动触发；不适用周期计划".to_string());
    }
    match task_key {
        "system_status_snapshot" => (
            "系统默认",
            "固定检查间隔：55 秒（60 秒缓存上限，提前 5 秒）".to_string(),
        ),
        "upstream_account_maintenance" => (
            "系统默认",
            "固定检查间隔：60 秒；账号同步按账号策略".to_string(),
        ),
        "pool_orphan_recovery" => ("系统默认", "固定检查间隔：60 秒".to_string()),
        "invocation_timeline_snapshot" => ("系统默认", "固定检查间隔：60 秒".to_string()),
        "dashboard_runtime_projection_reconcile" => (
            "系统默认",
            "固定检查间隔：60 秒；受压力准入约束".to_string(),
        ),
        "retention_archive" => (
            "运行配置",
            "启动检查与保留策略周期；按配置判断是否有工作".to_string(),
        ),
        "forward_proxy_subscription_refresh" => {
            ("系统默认", "启动刷新与固定检查间隔：60 秒".to_string())
        }
        "long_term_projection" => (
            "系统默认",
            "自适应检查；60 秒刷新、300 秒修复、每日校验".to_string(),
        ),
        "timeseries_minute_projection" => ("系统默认", "启动、固定检查与压力准入唤醒".to_string()),
        "summary_snapshot" => ("系统默认", "事件唤醒；最小刷新间隔 10 秒".to_string()),
        "summary_coverage_recovery" => ("系统默认", "自适应恢复；按覆盖和压力准入唤醒".to_string()),
        "prompt_cache_materialization" => ("系统默认", "事件对账与 60 秒检查".to_string()),
        "startup_backfill" => (
            "系统默认",
            "启动监督器；按事件、检查点和压力准入唤醒".to_string(),
        ),
        key if key.starts_with("startup_backfill.") => {
            ("系统默认", "回填父任务按事件和检查点调度".to_string())
        }
        key if key.contains("hourly") => ("启动规则", "仅在服务启动阶段检查".to_string()),
        _ => ("系统默认", "由 worker 的固定或事件规则检查".to_string()),
    }
}

fn decorate_task(mut task: ManagedTask) -> ManagedTask {
    let editable = EDITABLE_SCHEDULE_TASKS.contains(&task.task_key.as_str());
    let policy_interval_secs = (task.schedule_source.as_deref() != Some("default"))
        .then_some(task.interval_secs)
        .flatten();
    let (policy_source, effective_policy) = task_policy_text(
        &task.task_key,
        policy_interval_secs,
        task.cron_expr.as_deref(),
    );
    task.trigger_kinds = task_trigger_kinds(&task.task_key, &task.trigger_mode, task.is_manual);
    task.effective_policy = effective_policy;
    task.policy_source = policy_source.to_string();
    task.schedule_editable = editable;
    task.schedule_capability_reason = if task.is_manual {
        Some("手动任务没有周期或 cron 计划".to_string())
    } else if editable {
        None
    } else if task.interval_secs.is_some()
        || task
            .cron_expr
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        Some("当前 worker 为事件、启动或自适应路径，保留已有覆盖但不支持新增覆盖".to_string())
    } else {
        Some("当前任务只读展示真实 worker 策略".to_string())
    };
    task.execution_class = task_execution_class(&task.task_key).map(str::to_string);
    task
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
    ensure_task_colors(&pool).await?;
    Ok(MaintenanceStore { pool })
}

async fn ensure_task_colors(pool: &Pool<Sqlite>) -> Result<()> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT task_key,display_color_light,display_color_dark FROM managed_tasks ORDER BY task_key",
    )
    .fetch_all(pool)
    .await?;
    let mut used = HashSet::new();
    let mut next_hue = 0.0_f64;
    for (task_key, light, dark) in rows {
        if let (Some(light), Some(dark)) = (&light, &dark) {
            used.insert((light.clone(), dark.clone()));
            continue;
        }
        loop {
            let hue = next_hue % 360.0;
            next_hue += 137.507_764;
            let generated = (hsl_hex(hue, 0.72, 0.43), hsl_hex(hue, 0.76, 0.66));
            let colors = (
                light.clone().unwrap_or_else(|| generated.0.clone()),
                dark.clone().unwrap_or_else(|| generated.1.clone()),
            );
            if used.insert(colors.clone()) {
                sqlx::query(
                    "UPDATE managed_tasks SET display_color_light=COALESCE(display_color_light,?),display_color_dark=COALESCE(display_color_dark,?) WHERE task_key=?",
                )
                .bind(&generated.0)
                .bind(&generated.1)
                .bind(&task_key)
                .execute(pool)
                .await?;
                break;
            }
        }
    }
    Ok(())
}

fn hsl_hex(hue: f64, saturation: f64, lightness: f64) -> String {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match sector as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = lightness - chroma / 2.0;
    format!(
        "#{:02x}{:02x}{:02x}",
        ((red + offset) * 255.0).round() as u8,
        ((green + offset) * 255.0).round() as u8,
        ((blue + offset) * 255.0).round() as u8,
    )
}

fn validate_cron_expr(expr: Option<&str>) -> Result<()> {
    let Some(expr) = expr.map(str::trim) else {
        return Ok(());
    };
    if expr.is_empty() {
        return Err(anyhow!(
            "cron expression must contain exactly five UTC fields"
        ));
    }
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
            return (value - minimum).is_multiple_of(step);
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
                && (value - start).is_multiple_of(step);
        }
        base.parse::<u32>()
            .is_ok_and(|exact| exact == value && exact >= minimum && exact <= maximum)
    })
}

fn cron_field_is_unrestricted(field: &str) -> bool {
    field == "*"
}

fn cron_day_matches(dom_field: &str, dow_field: &str, candidate: chrono::DateTime<Utc>) -> bool {
    let dom_unrestricted = cron_field_is_unrestricted(dom_field);
    let dow_unrestricted = cron_field_is_unrestricted(dow_field);
    let dom_match = cron_field_matches(dom_field, candidate.day(), 1, 31);
    let dow_match = cron_field_matches(dow_field, candidate.weekday().num_days_from_sunday(), 0, 6);
    match (dom_unrestricted, dow_unrestricted) {
        (true, true) => true,
        (true, false) => dow_match,
        (false, true) => dom_match,
        (false, false) => dom_match || dow_match,
    }
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
        for offset in 1..=(5 * 366 * 24 * 60) {
            let candidate = base + ChronoDuration::minutes(offset);
            if cron_field_matches(fields[0], candidate.minute(), 0, 59)
                && cron_field_matches(fields[1], candidate.hour(), 0, 23)
                && cron_field_matches(fields[3], candidate.month(), 1, 12)
                && cron_day_matches(fields[2], fields[4], candidate)
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

fn decorate_effective_schedule(mut task: ManagedTask) -> ManagedTask {
    if task.is_manual {
        return task;
    }
    let (source, interval_secs, cron_expr) = if task.schedule_source.as_deref() == Some("default") {
        ("default", task.interval_secs, task.cron_expr.clone())
    } else if task
        .cron_expr
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || task.interval_secs.is_some()
    {
        ("override", task.interval_secs, task.cron_expr.clone())
    } else {
        return task;
    };
    task.effective_schedule = Some(ManagedTaskSchedule {
        source: source.to_string(),
        interval_secs,
        cron_expr: cron_expr.clone(),
        next_trigger_at: task
            .next_trigger_at
            .clone()
            .or_else(|| next_trigger_at(interval_secs, cron_expr.as_deref())),
    });
    task
}

fn decode_task_stages(raw: Option<&str>) -> Option<Vec<TaskStage>> {
    raw.and_then(|value| serde_json::from_str(value).ok())
}

fn task_progress_from_row(row: TaskProgressRow) -> TaskProgress {
    let freshness = if row.freshness == "fresh"
        && row.updated_at.as_deref().is_some_and(|updated_at| {
            chrono::DateTime::parse_from_rfc3339(updated_at)
                .ok()
                .is_some_and(|updated_at| {
                    Utc::now()
                        .signed_duration_since(updated_at.with_timezone(&Utc))
                        .num_seconds()
                        > TASK_PROGRESS_STALE_AFTER_SECS
                })
        }) {
        "stale".to_string()
    } else {
        row.freshness.clone()
    };
    TaskProgress {
        total: row.total,
        completed: row.completed,
        phase: row.phase,
        checkpoint: row.checkpoint,
        eta_seconds: row.eta_seconds,
        updated_at: row.updated_at,
        freshness,
        unit: row.unit,
        source_scope: row.source_scope,
        last_progress_at: row.last_progress_at,
        wait_reason: row.wait_reason,
        next_retry_at: row.next_retry_at,
        stages: decode_task_stages(row.stages.as_deref()),
    }
}

fn task_run_from_row(row: TaskRunRow) -> TaskRun {
    TaskRun {
        id: row.id,
        trigger_kind: row.trigger_kind,
        started_at: row.started_at,
        finished_at: row.finished_at,
        duration_ms: row.duration_ms,
        status: row.status,
        summary: row.summary,
        processed_count: row.processed_count,
        updated_count: row.updated_count,
        error_detail: row.error_detail,
        completion: row.completion,
        core_completion: row.core_completion,
        details: row
            .details
            .and_then(|value| serde_json::from_str(&value).ok()),
    }
}

pub(crate) fn managed_startup_backfill_suffix(task_name: &str) -> Option<&'static str> {
    if task_name == crate::STARTUP_BACKFILL_TASK_PROXY_COST
        || task_name
            .strip_prefix(crate::STARTUP_BACKFILL_TASK_PROXY_COST)
            .is_some_and(|suffix| suffix.starts_with(':'))
    {
        return Some("proxy_cost");
    }

    [
        (crate::STARTUP_BACKFILL_TASK_PROXY_USAGE, "proxy_usage"),
        (
            crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY,
            "prompt_cache_key",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION,
            "prompt_cache_conversations_materialization",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER,
            "requested_service_tier",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER,
            "invocation_service_tier",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_REASONING_EFFORT,
            "reasoning_effort",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION,
            "failure_classification",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_LIVE,
            "pool_attempt_public_id_live",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_ARCHIVES,
            "pool_attempt_public_id_archives",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_LIVE,
            "upstream_activity_live",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES,
            "upstream_activity_archives",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_POOL_UPSTREAM_NODE_HEALTH_ARCHIVES,
            "pool_upstream_node_health_archives",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_ACCOUNT_ACTIVITY_V2_COVERAGE,
            "account_activity_v2_coverage",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_LEGACY_DETAIL_MIRRORS,
            "legacy_detail_mirrors",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_HISTORICAL_ROLLUPS,
            "historical_rollups",
        ),
    ]
    .into_iter()
    .find_map(|(legacy_name, managed_suffix)| (task_name == legacy_name).then_some(managed_suffix))
}

async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    for statement in r#"
        CREATE TABLE IF NOT EXISTS managed_tasks (
          task_key TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL,
          trigger_mode TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
          interval_secs INTEGER, cron_expr TEXT, next_trigger_at TEXT,
          is_manual INTEGER NOT NULL DEFAULT 0, schedule_source TEXT,
          display_color_light TEXT, display_color_dark TEXT,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS managed_task_progress (
          task_key TEXT PRIMARY KEY REFERENCES managed_tasks(task_key) ON DELETE CASCADE,
          total INTEGER, completed INTEGER, phase TEXT, checkpoint TEXT, eta_seconds INTEGER,
          updated_at TEXT, freshness TEXT NOT NULL DEFAULT 'fresh', unit TEXT, source_scope TEXT,
          last_progress_at TEXT, wait_reason TEXT, next_retry_at TEXT, stages TEXT
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
          processed_count INTEGER, updated_count INTEGER, error_detail TEXT,
          completion TEXT, core_completion TEXT, details TEXT,
          execution_uid TEXT, actual_started_at TEXT, actual_finished_at TEXT,
          actual_duration_ms INTEGER
        );
        CREATE TABLE IF NOT EXISTS task_timeline_segments (
          segment_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, kind TEXT NOT NULL,
          task_key TEXT NOT NULL, title TEXT NOT NULL, started_at TEXT NOT NULL,
          last_observed_at TEXT NOT NULL, finished_at TEXT, duration_ms INTEGER,
          status TEXT NOT NULL, trigger_kind TEXT, execution_class TEXT,
          reason TEXT, retry_at TEXT, active_child_task_key TEXT, active_child_title TEXT,
          managed_run_id INTEGER, revision INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_task_timeline_segments_started
          ON task_timeline_segments(started_at, segment_id);
        CREATE INDEX IF NOT EXISTS idx_task_timeline_segments_open
          ON task_timeline_segments(kind, status, task_key);
        CREATE TABLE IF NOT EXISTS task_timeline_coverage (
          session_id TEXT PRIMARY KEY, started_at TEXT NOT NULL, last_seen_at TEXT NOT NULL,
          ended_at TEXT, dropped_events INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_managed_task_runs_task_started
          ON managed_task_runs(task_key, started_at DESC);
        CREATE TABLE IF NOT EXISTS maintenance_metadata (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
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
        ("completion", "TEXT"),
        ("core_completion", "TEXT"),
        ("details", "TEXT"),
        ("execution_uid", "TEXT"),
        ("actual_started_at", "TEXT"),
        ("actual_finished_at", "TEXT"),
        ("actual_duration_ms", "INTEGER"),
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
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_managed_task_runs_execution_uid ON managed_task_runs(execution_uid)",
    )
    .execute(pool)
    .await?;
    for (column, definition) in [
        ("unit", "TEXT"),
        ("source_scope", "TEXT"),
        ("last_progress_at", "TEXT"),
        ("wait_reason", "TEXT"),
        ("next_retry_at", "TEXT"),
        ("stages", "TEXT"),
    ] {
        let present: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT 1 FROM pragma_table_info('managed_task_progress') WHERE name = '{column}'"
        ))
        .fetch_optional(pool)
        .await?;
        if present.is_none() {
            sqlx::query(&format!(
                "ALTER TABLE managed_task_progress ADD COLUMN {column} {definition}"
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
    let has_schedule_source: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('managed_tasks') WHERE name = 'schedule_source'",
    )
    .fetch_optional(pool)
    .await?;
    if has_schedule_source.is_none() {
        sqlx::query("ALTER TABLE managed_tasks ADD COLUMN schedule_source TEXT")
            .execute(pool)
            .await?;
    }
    for (column, definition) in [
        ("display_color_light", "TEXT"),
        ("display_color_dark", "TEXT"),
    ] {
        let present: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT 1 FROM pragma_table_info('managed_tasks') WHERE name = '{column}'"
        ))
        .fetch_optional(pool)
        .await?;
        if present.is_none() {
            sqlx::query(&format!(
                "ALTER TABLE managed_tasks ADD COLUMN {column} {definition}"
            ))
            .execute(pool)
            .await?;
        }
    }
    // Keep databases written by the old dual-field form deterministic: an explicit cron wins.
    // Recompute the persisted trigger at the same time; an interval-derived timestamp is not
    // valid once the cron field becomes authoritative.
    let legacy_dual_schedule_rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT task_key,cron_expr FROM managed_tasks WHERE cron_expr IS NOT NULL AND trim(cron_expr) <> '' AND interval_secs IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    sqlx::query(
        "UPDATE managed_tasks SET interval_secs=NULL WHERE cron_expr IS NOT NULL AND trim(cron_expr) <> '' AND interval_secs IS NOT NULL",
    )
    .execute(pool)
    .await?;
    for (task_key, cron_expr) in legacy_dual_schedule_rows {
        sqlx::query("UPDATE managed_tasks SET next_trigger_at=? WHERE task_key=?")
            .bind(next_trigger_at(None, cron_expr.as_deref()))
            .bind(task_key)
            .execute(pool)
            .await?;
    }
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_task_runs_legacy_id ON managed_task_runs(legacy_id) WHERE legacy_id IS NOT NULL")
        .execute(pool)
        .await?;
    let superseded_at = format_utc_iso_millis(Utc::now());
    sqlx::query(
        "UPDATE managed_task_runs
         SET status='failed', finished_at=COALESCE(finished_at, ?1), duration_ms=COALESCE(duration_ms, 0),
             summary=COALESCE(summary, '重复的活动运行已停止'),
             error_detail=COALESCE(error_detail, '迁移时保留较早活动运行，重复记录已终止')
         WHERE id IN (
             SELECT duplicate.id
             FROM managed_task_runs duplicate
             JOIN managed_task_runs kept
               ON kept.task_key = duplicate.task_key
              AND kept.status IN ('running','requested')
              AND duplicate.status IN ('running','requested')
              AND kept.id < duplicate.id
         )",
    )
    .bind(&superseded_at)
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
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,enabled,is_manual,updated_at) VALUES (?,?,?,?,?,?,?) ON CONFLICT(task_key) DO UPDATE SET title=excluded.title, description=excluded.description, trigger_mode=excluded.trigger_mode, is_manual=excluded.is_manual")
            .bind(key).bind(title).bind(description).bind(mode).bind(task_enabled_by_default(key, *manual) as i64).bind(*manual as i64).bind(&now).execute(pool).await?;
    }
    for key in STARTUP_BACKFILL_TASKS {
        let (title, description) = startup_backfill_task_metadata(key);
        let task_key = format!("startup_backfill.{key}");
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,enabled,is_manual,updated_at) VALUES (?,?,?,?,?,?,?) ON CONFLICT(task_key) DO UPDATE SET title=excluded.title, description=excluded.description, trigger_mode=excluded.trigger_mode, is_manual=excluded.is_manual")
            .bind(&task_key).bind(title).bind(description).bind("event").bind(task_enabled_by_default(&task_key, false) as i64).bind(0_i64).bind(&now).execute(pool).await?;
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
    pub(crate) async fn start_timeline_session(
        &self,
        session_id: &str,
        started_at: &str,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO maintenance_metadata(key,value,updated_at) VALUES('task_timeline_revision','0',?)")
            .bind(started_at)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE maintenance_metadata SET value=CAST(value AS INTEGER)+1,updated_at=? WHERE key='task_timeline_revision'")
            .bind(started_at)
            .execute(&mut *transaction)
            .await?;
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT CAST(value AS INTEGER) FROM maintenance_metadata WHERE key='task_timeline_revision'",
        )
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE task_timeline_segments SET status='interrupted',revision=? WHERE status IN ('running','waiting')",
        )
        .bind(revision)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE task_timeline_coverage SET ended_at=last_seen_at WHERE ended_at IS NULL",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO task_timeline_coverage(session_id,started_at,last_seen_at,dropped_events) VALUES(?,?,?,0) ON CONFLICT(session_id) DO UPDATE SET started_at=excluded.started_at,last_seen_at=excluded.last_seen_at,ended_at=NULL,dropped_events=0",
        )
        .bind(session_id)
        .bind(started_at)
        .bind(started_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn write_timeline_batch(
        &self,
        session_id: &str,
        events: &[crate::task_timeline::TimelineEvent],
        dropped_events: u64,
        observed_at: &str,
        heartbeat: bool,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO maintenance_metadata(key,value,updated_at) VALUES('task_timeline_revision','0',?)")
            .bind(observed_at)
            .execute(&mut *transaction)
            .await?;
        let prior_dropped = sqlx::query_scalar::<_, i64>(
            "SELECT dropped_events FROM task_timeline_coverage WHERE session_id=?",
        )
        .bind(session_id)
        .fetch_optional(&mut *transaction)
        .await?
        .unwrap_or(0);
        let dropped_events = dropped_events.min(i64::MAX as u64) as i64;
        let revision_increments =
            (!events.is_empty() || heartbeat || dropped_events > prior_dropped) as i64;
        if revision_increments > 0 {
            sqlx::query("UPDATE maintenance_metadata SET value=CAST(value AS INTEGER)+?,updated_at=? WHERE key='task_timeline_revision'")
                .bind(revision_increments)
                .bind(observed_at)
                .execute(&mut *transaction)
                .await?;
        }
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT CAST(value AS INTEGER) FROM maintenance_metadata WHERE key='task_timeline_revision'",
        )
        .fetch_one(&mut *transaction)
        .await?;
        for event in events {
            match event {
                crate::task_timeline::TimelineEvent::ExecutionStarted {
                    id,
                    task_key,
                    title,
                    trigger_kind,
                    execution_class,
                    started_at,
                    managed_run_id,
                } => {
                    sqlx::query("INSERT OR IGNORE INTO task_timeline_segments(segment_id,session_id,kind,task_key,title,started_at,last_observed_at,status,trigger_kind,execution_class,managed_run_id,revision) VALUES(?,?,'execution',?,?,?,?, 'running',?,?,?,?)")
                        .bind(id).bind(session_id).bind(task_key).bind(title).bind(started_at)
                        .bind(started_at).bind(trigger_kind).bind(execution_class)
                        .bind(managed_run_id).bind(revision)
                        .execute(&mut *transaction).await?;
                    if let Some(run_id) = managed_run_id {
                        sqlx::query("UPDATE managed_task_runs SET execution_uid=?,actual_started_at=? WHERE id=?")
                            .bind(id).bind(started_at).bind(run_id)
                            .execute(&mut *transaction).await?;
                    }
                }
                crate::task_timeline::TimelineEvent::ExecutionFinished {
                    id,
                    finished_at,
                    duration_ms,
                    status,
                } => {
                    sqlx::query("UPDATE task_timeline_segments SET finished_at=?,last_observed_at=?,duration_ms=?,status=?,revision=? WHERE segment_id=? AND kind='execution'")
                        .bind(finished_at).bind(finished_at).bind((*duration_ms).min(i64::MAX as u64) as i64)
                        .bind(status).bind(revision).bind(id)
                        .execute(&mut *transaction).await?;
                    sqlx::query("UPDATE managed_task_runs SET actual_finished_at=?,actual_duration_ms=? WHERE execution_uid=?")
                        .bind(finished_at).bind((*duration_ms).min(i64::MAX as u64) as i64).bind(id)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::ExecutionUnknown {
                    id,
                    last_observed_at,
                } => {
                    sqlx::query("UPDATE task_timeline_segments SET finished_at=NULL,last_observed_at=?,duration_ms=NULL,status='unknown',revision=? WHERE segment_id=? AND kind='execution'")
                        .bind(last_observed_at).bind(revision).bind(id)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::ExecutionChildChanged {
                    id,
                    task_key,
                    title,
                } => {
                    sqlx::query("UPDATE task_timeline_segments SET active_child_task_key=?,active_child_title=?,revision=? WHERE segment_id=? AND kind='execution' AND status='running'")
                        .bind(task_key).bind(title).bind(revision).bind(id)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::CoverageGap {
                    id,
                    started_at,
                    finished_at,
                    reason,
                } => {
                    sqlx::query("INSERT OR IGNORE INTO task_timeline_segments(segment_id,session_id,kind,task_key,title,started_at,last_observed_at,finished_at,status,reason,revision) VALUES(?,?,'coverage_gap','__timeline__','观测缺口',?,?,?,'unknown',?,?)")
                        .bind(id).bind(session_id).bind(started_at).bind(finished_at)
                        .bind(finished_at).bind(reason).bind(revision)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::DeferralStarted {
                    id,
                    task_key,
                    reason,
                    retry_at,
                    started_at,
                } => {
                    let title = task_title_for_observation(task_key);
                    sqlx::query("INSERT OR IGNORE INTO task_timeline_segments(segment_id,session_id,kind,task_key,title,started_at,last_observed_at,status,reason,retry_at,revision) VALUES(?,?,'deferral',?,?,?,?, 'waiting',?,?,?)")
                        .bind(id).bind(session_id).bind(task_key).bind(title).bind(started_at)
                        .bind(started_at).bind(reason).bind(retry_at).bind(revision)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::DeferralFinished { id, finished_at } => {
                    sqlx::query("UPDATE task_timeline_segments SET finished_at=?,last_observed_at=?,status='released',revision=? WHERE segment_id=? AND kind='deferral'")
                        .bind(finished_at).bind(finished_at).bind(revision).bind(id)
                        .execute(&mut *transaction).await?;
                }
            }
        }
        if heartbeat || !events.is_empty() || dropped_events > prior_dropped {
            sqlx::query("UPDATE task_timeline_coverage SET last_seen_at=?,dropped_events=MAX(dropped_events,?) WHERE session_id=?")
                .bind(observed_at).bind(dropped_events).bind(session_id)
                .execute(&mut *transaction).await?;
        }
        if heartbeat {
            sqlx::query("UPDATE task_timeline_segments SET last_observed_at=?,revision=? WHERE session_id=? AND status IN ('running','waiting')")
                .bind(observed_at).bind(revision).bind(session_id)
                .execute(&mut *transaction).await?;
        }
        let cutoff = format_utc_iso_millis(
            Utc::now() - ChronoDuration::hours(TASK_TIMELINE_RETENTION_HOURS),
        );
        sqlx::query("DELETE FROM task_timeline_segments WHERE status NOT IN ('running','waiting') AND COALESCE(finished_at,last_observed_at) < ?")
            .bind(&cutoff)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "DELETE FROM task_timeline_coverage WHERE ended_at IS NOT NULL AND ended_at < ?",
        )
        .bind(cutoff)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn finish_timeline_session(
        &self,
        session_id: &str,
        ended_at: &str,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE task_timeline_segments SET status='interrupted' WHERE session_id=? AND status IN ('running','waiting')")
            .bind(session_id).execute(&mut *transaction).await?;
        sqlx::query(
            "UPDATE task_timeline_coverage SET ended_at=?,last_seen_at=? WHERE session_id=?",
        )
        .bind(ended_at)
        .bind(ended_at)
        .bind(session_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn timeline_revision(&self) -> Result<i64> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT CAST(value AS INTEGER) FROM maintenance_metadata WHERE key='task_timeline_revision'",
        )
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0))
    }

    pub(crate) async fn list_timeline_segments(
        &self,
        from: &str,
        to: &str,
        watermark: i64,
        after_revision: Option<i64>,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<TimelineSegment>> {
        let rows = sqlx::query_as::<_, TimelineSegment>("SELECT segment_id,kind,task_key,title,started_at,last_observed_at,finished_at,duration_ms,status,trigger_kind,execution_class,reason,retry_at,active_child_task_key,active_child_title,managed_run_id,session_id,revision FROM task_timeline_segments WHERE started_at<=? AND COALESCE(finished_at,last_observed_at)>=? AND revision<=? AND (? IS NULL OR revision>?) ORDER BY started_at,segment_id LIMIT ? OFFSET ?")
            .bind(to).bind(from).bind(watermark).bind(after_revision).bind(after_revision)
            .bind(limit.min(500) as i64).bind(offset.min(i64::MAX as u64) as i64)
            .fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub(crate) async fn list_timeline_coverage(&self) -> Result<Vec<TimelineCoverage>> {
        Ok(sqlx::query_as::<_, TimelineCoverage>("SELECT session_id,started_at,last_seen_at,ended_at,dropped_events FROM task_timeline_coverage ORDER BY started_at DESC LIMIT 50")
            .fetch_all(&self.pool).await?)
    }

    pub(crate) async fn list_queued_runs(&self) -> Result<Vec<QueuedTaskRun>> {
        let rows = sqlx::query_as::<_, (i64, String, String, String, String)>("SELECT r.id,r.task_key,COALESCE(t.title,r.task_key),r.trigger_kind,r.started_at FROM managed_task_runs r LEFT JOIN managed_tasks t ON t.task_key=r.task_key WHERE r.status='requested' ORDER BY r.id LIMIT 500")
            .fetch_all(&self.pool).await?;
        let now = Utc::now();
        Ok(rows
            .into_iter()
            .enumerate()
            .map(
                |(index, (run_id, task_key, title, trigger_kind, requested_at))| {
                    let waiting_ms = chrono::DateTime::parse_from_rfc3339(&requested_at)
                        .map(|started| {
                            (now - started.with_timezone(&Utc))
                                .num_milliseconds()
                                .max(0)
                        })
                        .unwrap_or(0);
                    QueuedTaskRun {
                        run_id,
                        task_key,
                        title,
                        trigger_kind,
                        requested_at,
                        waiting_ms,
                        position: index as i64 + 1,
                    }
                },
            )
            .collect())
    }

    pub(crate) async fn list_current_task_deferrals(&self) -> Result<Vec<TaskAdmissionWait>> {
        let freshness_cutoff = format_utc_iso_millis(Utc::now() - ChronoDuration::seconds(60));
        let session_id = sqlx::query_scalar::<_, String>(
            "SELECT session_id FROM task_timeline_coverage WHERE ended_at IS NULL AND last_seen_at>=? ORDER BY started_at DESC LIMIT 1",
        )
        .bind(&freshness_cutoff)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("task timeline observation is unavailable"))?;
        let rows = sqlx::query_as::<_, (String, String, String, String, String, Option<String>)>("SELECT segment_id,task_key,title,reason,started_at,retry_at FROM task_timeline_segments WHERE kind='deferral' AND status='waiting' AND session_id=? AND last_observed_at>=? ORDER BY started_at,segment_id")
            .bind(session_id)
            .bind(&freshness_cutoff)
            .fetch_all(&self.pool).await?;
        let now = Utc::now();
        Ok(rows
            .into_iter()
            .map(|(id, task_key, title, reason, started_at, retry_at)| {
                let waiting_ms = chrono::DateTime::parse_from_rfc3339(&started_at)
                    .map(|started| {
                        (now - started.with_timezone(&Utc))
                            .num_milliseconds()
                            .max(0)
                    })
                    .unwrap_or(0);
                TaskAdmissionWait {
                    id,
                    task_key,
                    title,
                    reason,
                    started_at,
                    waiting_ms,
                    retry_at,
                }
            })
            .collect())
    }

    pub(crate) async fn claim_requested_run(
        &self,
    ) -> Result<Option<(i64, String, String, String)>> {
        let mut transaction = self.pool.begin().await?;
        let finished_at = format_utc_iso_millis(Utc::now());
        sqlx::query(
            "UPDATE managed_task_runs
             SET status='failed', finished_at=?, duration_ms=0,
                 summary='任务已停用，未执行', error_detail=NULL
             WHERE status='requested'
               AND task_key IN (
                   SELECT task_key FROM managed_tasks WHERE enabled=0 AND is_manual=0
               )",
        )
        .bind(&finished_at)
        .execute(&mut *transaction)
        .await?;
        let claimed = sqlx::query_as::<_, (i64, String, String, String)>(
            "UPDATE managed_task_runs
             SET status='running'
             WHERE id = (
                 SELECT id FROM managed_task_runs
                 WHERE status='requested'
                   AND task_key IN (
                       SELECT task_key FROM managed_tasks WHERE enabled!=0 OR is_manual!=0
                   )
                 ORDER BY id
                 LIMIT 1
             )
             AND status='requested'
             RETURNING id,task_key,started_at,trigger_kind",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((id, task_key, started_at, trigger_kind)) = claimed else {
            transaction.commit().await?;
            return Ok(None);
        };
        transaction.commit().await?;
        Ok(Some((id, task_key, started_at, trigger_kind)))
    }

    pub(crate) async fn recover_incomplete_runs(&self) -> Result<u64> {
        let recovery_cutoff = format_utc_iso_millis(
            Utc::now() - ChronoDuration::seconds(TASK_RUNNING_RECOVERY_STALE_AFTER_SECS),
        );
        let finished_at = format_utc_iso_millis(Utc::now());
        let result = sqlx::query(
            "UPDATE managed_task_runs
             SET status='failed', finished_at=?, duration_ms=0,
                 summary=COALESCE(summary, ?),
                 error_detail=COALESCE(error_detail, ?)
             WHERE status='running' AND started_at < ?",
        )
        .bind(&finished_at)
        .bind("服务重启前运行未完成，已标记为失败")
        .bind("服务重启时回收未完成运行")
        .bind(&recovery_cutoff)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
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
        let result = sqlx::query_scalar(
            "INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary,error_detail)
             SELECT task_key,'manual',?,'requested','手动运行请求',NULL
             FROM managed_tasks
             WHERE task_key=? AND (enabled!=0 OR is_manual!=0)
             RETURNING id",
        )
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(task_key)
        .fetch_optional(&self.pool)
        .await;
        let result = match result {
            Ok(Some(run_id)) => Ok(run_id),
            Ok(None) => Err(anyhow!("task is disabled; enable it before run-now")),
            Err(error) => Err(error.into()),
        };
        result.map_err(|error| {
            if error
                .to_string()
                .contains("UNIQUE constraint failed: managed_task_runs.task_key")
            {
                anyhow!("task already has an active run")
            } else {
                error
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
            let was_active = matches!(status.as_str(), "running" | "requested");
            let migrated_status = if was_active {
                "failed"
            } else {
                status.as_str()
            };
            let migrated_summary = if was_active {
                Some(
                    summary
                        .as_deref()
                        .unwrap_or("服务重启前运行未完成，已标记为失败"),
                )
            } else {
                summary.as_deref()
            };
            let sanitized_detail = detail.as_deref().map(sanitize_task_detail);
            let migrated_detail = if was_active {
                Some(
                    sanitized_detail
                        .as_deref()
                        .unwrap_or("服务重启前运行未完成"),
                )
            } else {
                sanitized_detail.as_deref()
            };
            let migrated_finished_at = if was_active {
                Some(format_utc_iso_millis(Utc::now()))
            } else {
                finished_at
            };
            let migrated_duration_ms = if was_active { Some(0) } else { duration_ms };
            sqlx::query(
                "INSERT OR IGNORE INTO managed_task_runs (legacy_id,task_key,trigger_kind,started_at,finished_at,duration_ms,status,summary,error_detail) VALUES (?,?,?,?,?,?,?,?,?)",
            )
            .bind(id)
            .bind(task_key)
            .bind(trigger_kind)
            .bind(started_at)
            .bind(migrated_finished_at)
            .bind(migrated_duration_ms)
            .bind(migrated_status)
            .bind(migrated_summary)
            .bind(migrated_detail)
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
            let Some(managed_suffix) = managed_startup_backfill_suffix(&task_name) else {
                // Keep versioned or catalog-specific legacy rows without inventing a page.
                continue;
            };
            let task_key = format!("startup_backfill.{managed_suffix}");
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

    pub(crate) async fn apply_initial_task_defaults(&self) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let now = format_utc_iso_millis(Utc::now());
        let mut changed = false;
        let retention_schedule_applied: Option<String> =
            sqlx::query_scalar("SELECT value FROM maintenance_metadata WHERE key=?")
                .bind(RETENTION_DEFAULT_SCHEDULE_MARKER)
                .fetch_optional(&mut *transaction)
                .await?;
        if retention_schedule_applied.is_none() {
            sqlx::query(
                "UPDATE managed_tasks
                 SET interval_secs=?,
                     next_trigger_at=CASE WHEN enabled!=0 THEN COALESCE(next_trigger_at, ?) ELSE NULL END,
                     schedule_source='default',
                     updated_at=?
                 WHERE task_key='retention_archive'
                   AND is_manual=0
                   AND interval_secs IS NULL
                   AND (cron_expr IS NULL OR trim(cron_expr)='')",
            )
            .bind(DEFAULT_RETENTION_INTERVAL_SECS)
            .bind(&now)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
            sqlx::query("INSERT INTO maintenance_metadata (key,value,updated_at) VALUES (?,?,?)")
                .bind(RETENTION_DEFAULT_SCHEDULE_MARKER)
                .bind("applied")
                .bind(&now)
                .execute(&mut *transaction)
                .await?;
            changed = true;
        }
        let defaults_applied: Option<String> =
            sqlx::query_scalar("SELECT value FROM maintenance_metadata WHERE key=?")
                .bind(INITIAL_TASK_DEFAULTS_MARKER)
                .fetch_optional(&mut *transaction)
                .await?;
        let legacy_enablement_reconciled: Option<String> =
            sqlx::query_scalar("SELECT value FROM maintenance_metadata WHERE key=?")
                .bind(LEGACY_BACKFILL_ENABLEMENT_MARKER)
                .fetch_optional(&mut *transaction)
                .await?;
        if defaults_applied.is_some() && legacy_enablement_reconciled.is_some() {
            transaction.commit().await?;
            return Ok(changed);
        }

        // `seed_tasks` applies defaults only when a task row is first created. Do not
        // rewrite existing controls here: an upgrade must preserve operator choices.
        let disabled_until =
            format_utc_iso_millis(Utc::now() + ChronoDuration::days(TASK_DISABLED_UNTIL_DAYS));
        for task_name in [
            crate::STARTUP_BACKFILL_TASK_PROXY_USAGE,
            crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY,
            crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION,
            crate::STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER,
            crate::STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER,
            crate::STARTUP_BACKFILL_TASK_PROXY_COST,
            crate::STARTUP_BACKFILL_TASK_REASONING_EFFORT,
            crate::STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION,
            crate::STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_LIVE,
            crate::STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_ARCHIVES,
            crate::STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_LIVE,
            crate::STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES,
            crate::STARTUP_BACKFILL_TASK_POOL_UPSTREAM_NODE_HEALTH_ARCHIVES,
            crate::STARTUP_BACKFILL_TASK_ACCOUNT_ACTIVITY_V2_COVERAGE,
            crate::STARTUP_BACKFILL_TASK_LEGACY_DETAIL_MIRRORS,
            crate::STARTUP_BACKFILL_TASK_HISTORICAL_ROLLUPS,
        ] {
            let like_pattern = format!("{task_name}:%");
            let Some(managed_suffix) = managed_startup_backfill_suffix(task_name) else {
                continue;
            };
            let managed_key = format!("startup_backfill.{managed_suffix}");
            // The legacy progress row is the only durable enablement source before this
            // catalog exists. Seed the managed row from it once, then let the managed row
            // remain authoritative for later operator changes.
            sqlx::query(
                "UPDATE managed_tasks
                 SET enabled = COALESCE(
                     (SELECT MAX(enabled) FROM startup_backfill_progress
                      WHERE task_name=? OR task_name LIKE ?),
                     enabled
                 )
                 WHERE task_key=?",
            )
            .bind(task_name)
            .bind(&like_pattern)
            .bind(&managed_key)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "UPDATE startup_backfill_progress
                 SET enabled=COALESCE((SELECT enabled FROM managed_tasks WHERE task_key=?), 0),
                     next_run_after=CASE WHEN COALESCE((SELECT enabled FROM managed_tasks WHERE task_key=?), 0)=0 THEN ? ELSE next_run_after END,
                     suspension_reason=CASE WHEN COALESCE((SELECT enabled FROM managed_tasks WHERE task_key=?), 0)=0 THEN 'operator_disabled' ELSE suspension_reason END,
                     next_probe_at=CASE WHEN COALESCE((SELECT enabled FROM managed_tasks WHERE task_key=?), 0)=0 THEN NULL ELSE next_probe_at END
                 WHERE task_name=? OR task_name LIKE ?",
            )
            .bind(&managed_key)
            .bind(&managed_key)
            .bind(&disabled_until)
            .bind(&managed_key)
            .bind(&managed_key)
            .bind(task_name)
            .bind(like_pattern)
            .execute(&mut *transaction)
            .await?;
        }
        if defaults_applied.is_none() {
            sqlx::query(
                "INSERT OR IGNORE INTO maintenance_metadata (key,value,updated_at) VALUES (?,?,?)",
            )
            .bind(INITIAL_TASK_DEFAULTS_MARKER)
            .bind("applied")
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        if legacy_enablement_reconciled.is_none() {
            sqlx::query(
                "INSERT OR IGNORE INTO maintenance_metadata (key,value,updated_at) VALUES (?,?,?)",
            )
            .bind(LEGACY_BACKFILL_ENABLEMENT_MARKER)
            .bind("applied")
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(true)
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn publish_progress(
        &self,
        task_key: &str,
        total: Option<i64>,
        completed: Option<i64>,
        phase: Option<&str>,
        checkpoint: Option<&str>,
        unit: Option<&str>,
        source_scope: Option<&str>,
        wait_reason: Option<&str>,
        next_retry_at: Option<&str>,
        stages: Option<&[TaskStage]>,
    ) -> Result<()> {
        let now = format_utc_iso_millis(Utc::now());
        let stages = stages.map(serde_json::to_string).transpose()?;
        sqlx::query(
            "INSERT INTO managed_task_progress(task_key,total,completed,phase,checkpoint,updated_at,freshness,unit,source_scope,last_progress_at,wait_reason,next_retry_at,stages)
             VALUES(?,?,?,?,?,?,'fresh',?,?,?,?,?,?)
             ON CONFLICT(task_key) DO UPDATE SET total=excluded.total,completed=excluded.completed,phase=excluded.phase,checkpoint=excluded.checkpoint,updated_at=excluded.updated_at,freshness='fresh',unit=excluded.unit,source_scope=excluded.source_scope,last_progress_at=excluded.last_progress_at,wait_reason=excluded.wait_reason,next_retry_at=excluded.next_retry_at,stages=excluded.stages",
        )
        .bind(task_key)
        .bind(total)
        .bind(completed)
        .bind(phase)
        .bind(checkpoint)
        .bind(&now)
        .bind(unit)
        .bind(source_scope)
        .bind(&now)
        .bind(wait_reason)
        .bind(next_retry_at)
        .bind(stages)
        .execute(&self.pool)
        .await?;
        Ok(())
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
        self.finish_run_with_observation(
            id,
            status,
            finished_at,
            duration_ms,
            summary,
            detail,
            None,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn finish_run_with_observation(
        &self,
        id: i64,
        status: &str,
        finished_at: &str,
        duration_ms: i64,
        summary: Option<&str>,
        detail: Option<&str>,
        completion: Option<&str>,
        core_completion: Option<&str>,
        details: Option<&serde_json::Value>,
    ) -> Result<()> {
        let sanitized = detail.map(sanitize_task_detail);
        let details = details.map(serde_json::to_string).transpose()?;
        let result = sqlx::query("UPDATE managed_task_runs SET status=?,finished_at=?,duration_ms=?,summary=COALESCE(?,summary),error_detail=?,completion=?,core_completion=?,details=? WHERE id=?")
            .bind(status)
            .bind(finished_at)
            .bind(duration_ms)
            .bind(summary)
            .bind(sanitized)
            .bind(completion)
            .bind(core_completion)
            .bind(details)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(anyhow!("managed task run {id} was not found"));
        }
        Ok(())
    }

    pub(crate) async fn list_tasks(&self) -> Result<Vec<ManagedTask>> {
        let tasks = sqlx::query_as::<_, ManagedTask>("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,next_trigger_at,is_manual,display_color_light,display_color_dark,schedule_source FROM managed_tasks ORDER BY task_key")
            .fetch_all(&self.pool).await?;
        Ok(tasks
            .into_iter()
            .map(decorate_effective_schedule)
            .map(decorate_task)
            .collect())
    }

    pub(crate) async fn detail(&self, task_key: &str) -> Result<Option<ManagedTaskDetail>> {
        let task = sqlx::query_as::<_, ManagedTask>("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,next_trigger_at,is_manual,display_color_light,display_color_dark,schedule_source FROM managed_tasks WHERE task_key=?")
        .bind(task_key).fetch_optional(&self.pool).await?;
        let Some(task) = task.map(decorate_effective_schedule).map(decorate_task) else {
            return Ok(None);
        };
        let progress = sqlx::query_as::<_, TaskProgressRow>("SELECT total,completed,phase,checkpoint,eta_seconds,updated_at,freshness,unit,source_scope,last_progress_at,wait_reason,next_retry_at,stages FROM managed_task_progress WHERE task_key=?")
            .bind(task_key).fetch_optional(&self.pool).await?.map(task_progress_from_row);
        let recent_runs = sqlx::query_as::<_, TaskRunRow>("SELECT id,trigger_kind,started_at,finished_at,duration_ms,status,summary,processed_count,updated_count,error_detail,completion,core_completion,details FROM managed_task_runs WHERE task_key=? ORDER BY started_at DESC LIMIT 10")
            .bind(task_key).fetch_all(&self.pool).await?.into_iter().map(task_run_from_row).collect();
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
        sqlx::query("UPDATE managed_tasks SET interval_secs=?, cron_expr=?, next_trigger_at=?, schedule_source='override', updated_at=? WHERE task_key=? AND is_manual=0")
        .bind(interval_secs).bind(cron_expr).bind(next_trigger_at).bind(format_utc_iso_millis(Utc::now())).bind(task_key).execute(&self.pool).await?;
        Ok(true)
    }

    pub(crate) async fn update_control(
        &self,
        task_key: &str,
        enabled: Option<bool>,
        interval_secs: Option<Option<i64>>,
        cron_expr: Option<Option<&str>>,
    ) -> Result<bool> {
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
        let update_schedule = interval_secs.is_some() || cron_expr.is_some();
        if update_schedule && is_manual {
            transaction.commit().await?;
            return Err(anyhow!("manual or unknown task cannot be scheduled"));
        }
        let next_enabled = enabled.unwrap_or(current_enabled);
        let interval_value_provided = interval_secs.is_some_and(|value| value.is_some());
        let cron_value_provided = cron_expr.is_some_and(|value| value.is_some());
        if interval_value_provided && cron_value_provided {
            transaction.commit().await?;
            return Err(anyhow!("interval and cron schedule are mutually exclusive"));
        }
        let next_interval = if let Some(value) = interval_secs {
            value
        } else if cron_value_provided {
            None
        } else {
            current_interval
        };
        let next_cron = if let Some(value) = cron_expr {
            value.map(str::to_owned)
        } else if interval_value_provided {
            None
        } else {
            current_cron
        };
        if update_schedule {
            if !EDITABLE_SCHEDULE_TASKS.contains(&task_key)
                && (next_interval.is_some()
                    || next_cron
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()))
            {
                transaction.commit().await?;
                return Err(anyhow!(
                    "task does not support a new interval or cron override"
                ));
            }
            Self::validate_schedule(next_interval, next_cron.as_deref()).await?;
        }
        let next_trigger_at = if next_enabled && !is_manual {
            next_trigger_at(next_interval, next_cron.as_deref())
        } else {
            None
        };
        sqlx::query(
            "UPDATE managed_tasks SET enabled=?,interval_secs=?,cron_expr=?,next_trigger_at=?,schedule_source=CASE WHEN ? THEN 'override' ELSE schedule_source END,updated_at=? WHERE task_key=?",
        )
        .bind(next_enabled as i64)
        .bind(next_interval)
        .bind(next_cron)
        .bind(next_trigger_at)
        .bind(update_schedule)
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(task_key)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub(crate) async fn restore_control_state(&self, task: &ManagedTask) -> Result<()> {
        sqlx::query(
            "UPDATE managed_tasks SET enabled=?,interval_secs=?,cron_expr=?,next_trigger_at=?,updated_at=? WHERE task_key=?",
        )
        .bind(task.enabled as i64)
        .bind(task.interval_secs)
        .bind(task.cron_expr.as_deref())
        .bind(task.next_trigger_at.as_deref())
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(&task.task_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

pub(crate) fn path(config: &AppConfig) -> PathBuf {
    config.maintenance_database_path()
}

#[cfg(test)]
mod tests {
    use crate::format_utc_iso_millis;
    use chrono::TimeZone;
    use chrono::{Duration as ChronoDuration, Timelike, Utc};
    use sqlx::SqlitePool;
    use std::collections::HashSet;

    use super::{
        MANAGED_TASKS, MaintenanceStore, STARTUP_BACKFILL_TASKS, cron_day_matches, ensure_schema,
        ensure_task_colors, next_trigger_at, sanitize_task_detail, seed_tasks,
        task_enabled_by_default, validate_cron_expr,
    };

    #[tokio::test]
    async fn schema_repair_preserves_duplicate_active_run_history() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect maintenance migration fixture");
        sqlx::query(
            "CREATE TABLE managed_tasks (
                task_key TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL,
                trigger_mode TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
                interval_secs INTEGER, cron_expr TEXT, next_trigger_at TEXT,
                is_manual INTEGER NOT NULL DEFAULT 0, schedule_source TEXT, updated_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create legacy task table");
        sqlx::query(
            "CREATE TABLE managed_task_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT, legacy_id INTEGER UNIQUE, task_key TEXT NOT NULL,
                trigger_kind TEXT NOT NULL DEFAULT 'unknown', started_at TEXT NOT NULL, finished_at TEXT,
                duration_ms INTEGER, status TEXT NOT NULL, summary TEXT, processed_count INTEGER,
                updated_count INTEGER, error_detail TEXT, completion TEXT, core_completion TEXT, details TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create legacy task-run table");
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,updated_at) VALUES ('retention_archive','Retention','Retention','interval','2026-10-01T00:00:00.000Z')")
            .execute(&pool)
            .await
            .expect("seed task");
        for status in ["running", "requested"] {
            sqlx::query("INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary) VALUES ('retention_archive','manual','2026-10-01T00:00:00.000Z',?,?)")
                .bind(status)
                .bind(format!("{status} run"))
                .execute(&pool)
                .await
                .expect("seed duplicate active run");
        }

        ensure_schema(&pool)
            .await
            .expect("repair maintenance schema");

        let rows: Vec<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id,status,finished_at,error_detail FROM managed_task_runs ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .expect("load repaired run history");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, "running");
        assert_eq!(rows[1].1, "failed");
        assert!(rows[1].2.is_some());
        assert!(rows[1].3.is_some());
        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM managed_task_runs WHERE task_key='retention_archive' AND status IN ('running','requested')",
        )
        .fetch_one(&pool)
        .await
        .expect("count active runs");
        assert_eq!(active_count, 1);
    }

    #[tokio::test]
    async fn timeline_upgrade_is_repeatable_and_preserves_task_controls_and_colors() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect timeline migration fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        sqlx::query("UPDATE managed_tasks SET enabled=0,interval_secs=123,cron_expr=NULL WHERE task_key='retention_archive'")
            .execute(&pool)
            .await
            .expect("set existing task control overrides");
        sqlx::query("UPDATE managed_tasks SET cron_expr='5 * * * *',interval_secs=NULL WHERE task_key='upstream_account_maintenance'")
            .execute(&pool)
            .await
            .expect("set a cron override");
        ensure_task_colors(&pool)
            .await
            .expect("persist task colors");
        let color_before: (String, String) = sqlx::query_as(
            "SELECT display_color_light,display_color_dark FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&pool)
        .await
        .expect("load persisted task colors");
        let distinct_colors: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT display_color_light || ':' || display_color_dark) FROM managed_tasks",
        )
        .fetch_one(&pool)
        .await
        .expect("count task colors");
        assert_eq!(
            distinct_colors,
            (MANAGED_TASKS.len() + STARTUP_BACKFILL_TASKS.len()) as i64
        );

        ensure_schema(&pool)
            .await
            .expect("repeat maintenance schema upgrade");
        seed_tasks(&pool).await.expect("repeat task registry seed");
        ensure_task_colors(&pool)
            .await
            .expect("repeat task color assignment");
        let task: (bool, Option<i64>, Option<String>, String, String) = sqlx::query_as(
            "SELECT enabled,interval_secs,cron_expr,display_color_light,display_color_dark FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated task");
        assert!(!task.0);
        assert_eq!(task.1, Some(123));
        assert_eq!(task.2, None);
        assert_eq!((task.3, task.4), color_before);
        let cron_override: (bool, Option<i64>, Option<String>) = sqlx::query_as(
            "SELECT enabled,interval_secs,cron_expr FROM managed_tasks WHERE task_key='upstream_account_maintenance'",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated cron override");
        assert!(cron_override.0);
        assert_eq!(cron_override.1, None);
        assert_eq!(cron_override.2.as_deref(), Some("5 * * * *"));
        let timeline_table_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='task_timeline_segments'",
        )
        .fetch_one(&pool)
        .await
        .expect("check timeline table");
        assert_eq!(timeline_table_exists, 1);
    }

    #[tokio::test]
    async fn timeline_history_records_actual_execution_and_restart_gaps() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect timeline history fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        ensure_task_colors(&pool)
            .await
            .expect("seed stable task colors");
        let store = MaintenanceStore { pool };
        sqlx::query("INSERT INTO managed_task_runs(task_key,trigger_kind,started_at,duration_ms,status) VALUES('retention_archive','manual','2026-10-02T00:00:00.000Z',777,'running')")
            .execute(&store.pool)
            .await
            .expect("insert legacy-semantics run");
        let run_id = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM managed_task_runs WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load managed run id");
        store
            .start_timeline_session("session-one", "2026-10-02T00:00:00.000Z")
            .await
            .expect("start first observation session");
        let events = [
            crate::task_timeline::TimelineEvent::ExecutionStarted {
                id: "execution-one".to_string(),
                task_key: "retention_archive".to_string(),
                title: "数据保留与归档".to_string(),
                trigger_kind: "manual".to_string(),
                execution_class: Some("maintenance_retention".to_string()),
                started_at: "2026-10-02T00:00:01.000Z".to_string(),
                managed_run_id: Some(run_id),
            },
            crate::task_timeline::TimelineEvent::DeferralStarted {
                id: "deferral-one".to_string(),
                task_key: "retention_archive".to_string(),
                reason: "pressure_cooldown".to_string(),
                retry_at: None,
                started_at: "2026-10-02T00:00:02.000Z".to_string(),
            },
        ];
        store
            .write_timeline_batch("session-one", &events, 0, "2026-10-02T00:00:03.000Z", false)
            .await
            .expect("write execution and deferral starts");
        let ends = [
            crate::task_timeline::TimelineEvent::ExecutionFinished {
                id: "execution-one".to_string(),
                finished_at: "2026-10-02T00:00:06.000Z".to_string(),
                duration_ms: 5_000,
                status: "failed".to_string(),
            },
            crate::task_timeline::TimelineEvent::DeferralFinished {
                id: "deferral-one".to_string(),
                finished_at: "2026-10-02T00:00:05.000Z".to_string(),
            },
        ];
        store
            .write_timeline_batch("session-one", &ends, 2, "2026-10-02T00:00:07.000Z", false)
            .await
            .expect("write execution and deferral finishes");
        let actual_fields: (String, i64, String, i64) = sqlx::query_as(
            "SELECT started_at,duration_ms,actual_started_at,actual_duration_ms FROM managed_task_runs WHERE id=?",
        )
        .bind(run_id)
        .fetch_one(&store.pool)
        .await
        .expect("read legacy and actual execution fields");
        assert_eq!(actual_fields.0, "2026-10-02T00:00:00.000Z");
        assert_eq!(actual_fields.1, 777);
        assert_eq!(actual_fields.2, "2026-10-02T00:00:01.000Z");
        assert_eq!(actual_fields.3, 5_000);

        let open_events = [
            crate::task_timeline::TimelineEvent::ExecutionStarted {
                id: "execution-unknown".to_string(),
                task_key: "retention_archive".to_string(),
                title: "数据保留与归档".to_string(),
                trigger_kind: "interval".to_string(),
                execution_class: None,
                started_at: "2026-10-02T00:00:08.000Z".to_string(),
                managed_run_id: None,
            },
            crate::task_timeline::TimelineEvent::ExecutionUnknown {
                id: "execution-unknown".to_string(),
                last_observed_at: "2026-10-02T00:00:09.000Z".to_string(),
            },
            crate::task_timeline::TimelineEvent::ExecutionStarted {
                id: "execution-open".to_string(),
                task_key: "pool_orphan_recovery".to_string(),
                title: "连接池孤儿记录恢复".to_string(),
                trigger_kind: "interval".to_string(),
                execution_class: None,
                started_at: "2026-10-02T00:00:08.000Z".to_string(),
                managed_run_id: None,
            },
            crate::task_timeline::TimelineEvent::DeferralStarted {
                id: "deferral-open".to_string(),
                task_key: "pool_orphan_recovery".to_string(),
                reason: "resource_busy".to_string(),
                retry_at: None,
                started_at: "2026-10-02T00:00:08.000Z".to_string(),
            },
            crate::task_timeline::TimelineEvent::CoverageGap {
                id: "write-gap".to_string(),
                started_at: "2026-10-02T00:00:09.000Z".to_string(),
                finished_at: "2026-10-02T00:00:10.000Z".to_string(),
                reason: "maintenance_store_write_unavailable".to_string(),
            },
        ];
        store
            .write_timeline_batch(
                "session-one",
                &open_events,
                2,
                "2026-10-02T00:00:09.000Z",
                false,
            )
            .await
            .expect("write open execution and deferral");
        store
            .start_timeline_session("session-two", "2026-10-02T00:00:20.000Z")
            .await
            .expect("recover open observations after restart");
        let recovered: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT segment_id,status,finished_at,last_observed_at FROM task_timeline_segments WHERE segment_id IN ('execution-open','deferral-open') ORDER BY segment_id",
        )
        .fetch_all(&store.pool)
        .await
        .expect("read restart-recovered observations");
        assert_eq!(recovered.len(), 2);
        assert!(recovered.iter().all(|row| row.1 == "interrupted"));
        assert!(recovered.iter().all(|row| row.2.is_none()));
        assert!(
            recovered
                .iter()
                .all(|row| row.3 == "2026-10-02T00:00:08.000Z")
        );
        let gap: (String, String, Option<String>, String) = sqlx::query_as(
            "SELECT kind,started_at,finished_at,reason FROM task_timeline_segments WHERE segment_id='write-gap'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read persisted maintenance-store gap");
        assert_eq!(gap.0, "coverage_gap");
        assert_eq!(gap.1, "2026-10-02T00:00:09.000Z");
        assert_eq!(gap.2.as_deref(), Some("2026-10-02T00:00:10.000Z"));
        assert_eq!(gap.3, "maintenance_store_write_unavailable");
        let unknown: (Option<String>, Option<i64>, String) = sqlx::query_as(
            "SELECT finished_at,duration_ms,status FROM task_timeline_segments WHERE segment_id='execution-unknown'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read unknown execution terminal");
        assert_eq!(unknown.0, None);
        assert_eq!(unknown.1, None);
        assert_eq!(unknown.2, "unknown");
    }

    #[tokio::test]
    async fn current_admission_waits_require_fresh_recorder_coverage() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect admission wait fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        ensure_task_colors(&pool).await.expect("seed task colors");
        let store = MaintenanceStore { pool };
        let now = Utc::now();
        let observed_at = format_utc_iso_millis(now);
        store
            .start_timeline_session("admission-session", &observed_at)
            .await
            .expect("start fresh recorder coverage");
        let wait = crate::task_timeline::TimelineEvent::DeferralStarted {
            id: "active-admission-wait".to_string(),
            task_key: "retention_archive".to_string(),
            reason: "pressure_cooldown".to_string(),
            retry_at: None,
            started_at: observed_at.clone(),
        };
        store
            .write_timeline_batch("admission-session", &[wait], 0, &observed_at, false)
            .await
            .expect("persist active admission wait");
        let waits = store
            .list_current_task_deferrals()
            .await
            .expect("read fresh admission wait");
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].id, "active-admission-wait");

        let stale_at = format_utc_iso_millis(now - ChronoDuration::minutes(2));
        sqlx::query(
            "UPDATE task_timeline_coverage SET last_seen_at=? WHERE session_id='admission-session'",
        )
        .bind(&stale_at)
        .execute(&store.pool)
        .await
        .expect("age recorder coverage");
        sqlx::query("UPDATE task_timeline_segments SET last_observed_at=? WHERE segment_id='active-admission-wait'")
            .bind(&stale_at)
            .execute(&store.pool)
            .await
            .expect("age the open wait interval");
        assert!(store.list_current_task_deferrals().await.is_err());
    }

    #[tokio::test]
    async fn timeline_pages_use_a_fixed_watermark_and_return_later_revisions_once() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect timeline paging fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        ensure_task_colors(&pool)
            .await
            .expect("seed stable task colors");
        let store = MaintenanceStore { pool };
        let now = Utc::now();
        let started = (0..3)
            .map(
                |index| crate::task_timeline::TimelineEvent::ExecutionStarted {
                    id: format!("page-run-{index}"),
                    task_key: "retention_archive".to_string(),
                    title: "数据保留与归档".to_string(),
                    trigger_kind: "interval".to_string(),
                    execution_class: None,
                    started_at: format_utc_iso_millis(
                        now - ChronoDuration::seconds(60 - index * 20),
                    ),
                    managed_run_id: None,
                },
            )
            .collect::<Vec<_>>();
        let observed_at = format_utc_iso_millis(now);
        store
            .start_timeline_session("paging-session", &observed_at)
            .await
            .expect("start timeline paging session");
        store
            .write_timeline_batch("paging-session", &started, 0, &observed_at, false)
            .await
            .expect("write timeline paging rows");

        let first = crate::task_timeline::timeline_page(&store, None, None, None, None, 2)
            .await
            .expect("read first fixed-watermark page");
        assert_eq!(first.segments.len(), 2);
        let cursor = first.next_cursor.clone().expect("first page cursor");
        let second =
            crate::task_timeline::timeline_page(&store, Some(&cursor), None, None, None, 2)
                .await
                .expect("read second fixed-watermark page");
        assert_eq!(second.segments.len(), 1);
        assert!(second.next_cursor.is_none());
        assert_eq!(first.watermark, second.watermark);
        let ids = first
            .segments
            .iter()
            .chain(&second.segments)
            .map(|segment| segment.segment_id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), 3);

        let finish = [crate::task_timeline::TimelineEvent::ExecutionFinished {
            id: "page-run-0".to_string(),
            finished_at: observed_at.clone(),
            duration_ms: 60_000,
            status: "success".to_string(),
        }];
        store
            .write_timeline_batch("paging-session", &finish, 0, &observed_at, false)
            .await
            .expect("finish one timeline row");
        let delta = crate::task_timeline::timeline_page(
            &store,
            None,
            Some(first.watermark),
            None,
            None,
            500,
        )
        .await
        .expect("read timeline revision delta");
        assert_eq!(delta.segments.len(), 1);
        assert_eq!(delta.segments[0].segment_id, "page-run-0");
        assert_eq!(delta.segments[0].status, "success");
        assert!(delta.watermark > first.watermark);
    }

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
    fn disables_backfill_and_manual_tasks_by_default() {
        assert!(!task_enabled_by_default("startup_backfill", false));
        assert!(!task_enabled_by_default(
            "startup_backfill.proxy_usage",
            false
        ));
        assert!(!task_enabled_by_default("raw_compression", true));
        assert!(task_enabled_by_default("retention_archive", false));
        assert_eq!(
            MANAGED_TASKS
                .iter()
                .filter(|(key, _, _, _, is_manual)| task_enabled_by_default(key, *is_manual))
                .count(),
            14
        );
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
    fn cron_day_fields_follow_unrestricted_and_restricted_semantics() {
        let monday = chrono::Utc.with_ymd_and_hms(2026, 10, 12, 0, 0, 0).unwrap();
        let tuesday = chrono::Utc.with_ymd_and_hms(2026, 10, 13, 0, 0, 0).unwrap();
        assert!(cron_day_matches("*", "1", monday));
        assert!(cron_day_matches("12", "*", monday));
        assert!(cron_day_matches("*/2", "1", tuesday));
    }

    #[test]
    fn computes_a_next_trigger_beyond_one_year() {
        let now = chrono::Utc::now();
        let next = next_trigger_at(None, Some("0 0 29 2 *"))
            .expect("a valid leap-day cron should have a future trigger");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next)
            .expect("next trigger should be RFC3339")
            .with_timezone(&chrono::Utc);
        assert!(parsed > now);
        assert!(parsed <= now + chrono::Duration::days(5 * 366));
    }

    #[test]
    fn rejects_cron_expressions_without_five_utc_fields() {
        assert!(validate_cron_expr(Some("*/5 * * *")).is_err());
        assert!(validate_cron_expr(Some("*/5 * * * *")).is_ok());
        assert!(validate_cron_expr(Some("   ")).is_err());
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

    #[tokio::test]
    async fn decorates_root_and_backfill_tasks_with_effective_policy_and_capability() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore { pool };

        let tasks = store.list_tasks().await.expect("list decorated tasks");
        assert_eq!(
            tasks.len(),
            MANAGED_TASKS.len() + STARTUP_BACKFILL_TASKS.len()
        );
        assert!(tasks.iter().all(|task| {
            !task.trigger_kinds.is_empty()
                && !task.effective_policy.is_empty()
                && !task.policy_source.is_empty()
        }));
        let status = tasks
            .iter()
            .find(|task| task.task_key == "system_status_snapshot")
            .expect("system status task");
        assert!(status.effective_policy.contains("55 秒"));
        assert!(status.schedule_editable);
        let child = tasks
            .iter()
            .find(|task| task.task_key == "startup_backfill.proxy_usage")
            .expect("backfill child task");
        assert_eq!(child.trigger_kinds, vec!["event", "interval"]);
        assert!(!child.schedule_editable);
        assert!(child.schedule_capability_reason.is_some());
    }

    #[tokio::test]
    async fn repairs_legacy_dual_schedule_trigger_on_schema_open() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance migration test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "UPDATE managed_tasks SET interval_secs=120, cron_expr='*/5 * * * *', next_trigger_at='2000-01-01T00:02:00.000Z' WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .execute(&pool)
        .await
        .expect("seed legacy dual schedule");

        ensure_schema(&pool).await.expect("repair legacy schedule");
        let row = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<String>)>(
            "SELECT interval_secs,cron_expr,next_trigger_at FROM managed_tasks WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .fetch_one(&pool)
        .await
        .expect("load repaired schedule");
        assert_eq!(row.0, None);
        assert_eq!(row.1.as_deref(), Some("*/5 * * * *"));
        assert_ne!(row.2.as_deref(), Some("2000-01-01T00:02:00.000Z"));
        assert!(row.2.is_some());
    }

    #[tokio::test]
    async fn preserves_enabled_when_clearing_unsupported_override_and_rejects_new_one() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "UPDATE managed_tasks SET enabled=1, interval_secs=120 WHERE task_key='summary_snapshot'",
        )
        .execute(&pool)
        .await
        .expect("seed legacy unsupported override");
        let store = MaintenanceStore { pool };

        let error = store
            .update_control("summary_snapshot", None, Some(Some(180)), None)
            .await
            .expect_err("unsupported task should reject a new override");
        assert!(error.to_string().contains("does not support"));
        assert!(
            store
                .update_control("summary_snapshot", None, Some(None), Some(None))
                .await
                .expect("clearing an existing override should succeed")
        );
        let row = sqlx::query_as::<_, (bool, Option<i64>, Option<String>)>(
            "SELECT enabled,interval_secs,cron_expr FROM managed_tasks WHERE task_key='summary_snapshot'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load cleared task control");
        assert_eq!(row, (true, None, None));
    }

    #[tokio::test]
    async fn switching_supported_schedule_types_clears_the_previous_override() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance schedule test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore { pool };

        store
            .update_control(
                "dashboard_runtime_projection_reconcile",
                None,
                Some(Some(120)),
                None,
            )
            .await
            .expect("set interval override");
        store
            .update_control(
                "dashboard_runtime_projection_reconcile",
                None,
                None,
                Some(Some("*/5 * * * *")),
            )
            .await
            .expect("switch to cron override");
        let cron_state = sqlx::query_as::<_, (Option<i64>, Option<String>)>(
            "SELECT interval_secs,cron_expr FROM managed_tasks WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load cron override");
        assert_eq!(cron_state, (None, Some("*/5 * * * *".to_string())));

        store
            .update_control(
                "dashboard_runtime_projection_reconcile",
                None,
                Some(Some(180)),
                None,
            )
            .await
            .expect("switch back to interval override");
        let interval_state = sqlx::query_as::<_, (Option<i64>, Option<String>)>(
            "SELECT interval_secs,cron_expr FROM managed_tasks WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load interval override");
        assert_eq!(interval_state, (Some(180), None));
    }

    #[tokio::test]
    async fn recovers_incomplete_runs_idempotently() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary) VALUES ('retention_archive','startup','2026-09-30T00:00:00.000Z','running','started')",
        )
        .execute(&pool)
        .await
        .expect("insert incomplete managed task run");
        let fresh_started_at = crate::format_utc_iso_millis(Utc::now());
        sqlx::query(
            "INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary) VALUES ('forward_proxy_subscription_refresh','startup',?1,'running','started')",
        )
        .bind(&fresh_started_at)
        .execute(&pool)
        .await
        .expect("insert fresh managed task run");

        let store = MaintenanceStore { pool };
        assert_eq!(store.recover_incomplete_runs().await.unwrap(), 1);
        let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT status,finished_at,error_detail FROM managed_task_runs WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load recovered managed task run");
        assert_eq!(row.0, "failed");
        assert!(row.1.is_some());
        assert_eq!(row.2.as_deref(), Some("服务重启时回收未完成运行"));
        let fresh_status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM managed_task_runs WHERE task_key='forward_proxy_subscription_refresh'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load fresh managed task run");
        assert_eq!(fresh_status, "running");
        assert_eq!(store.recover_incomplete_runs().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn rejects_run_now_for_disabled_scheduled_tasks_but_allows_manual_tasks() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore { pool };

        let error = store
            .request_run("startup_backfill.proxy_usage")
            .await
            .expect_err("disabled scheduled task should reject run-now");
        assert!(error.to_string().contains("task is disabled"));

        let run_id = store
            .request_run("raw_compression")
            .await
            .expect("manual task should allow run-now while disabled");
        assert!(run_id > 0);
    }

    #[tokio::test]
    async fn updates_canonical_prompt_cache_backfill_control() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore { pool };

        assert!(
            store
                .update_control(
                    "startup_backfill.prompt_cache_conversations_materialization",
                    Some(true),
                    None,
                    None,
                )
                .await
                .expect("enable canonical prompt-cache control")
        );
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='startup_backfill.prompt_cache_conversations_materialization'",
            )
            .fetch_one(&store.pool)
            .await
            .expect("load canonical prompt-cache control")
        );
    }

    #[tokio::test]
    async fn claim_retires_requested_runs_after_a_task_is_disabled() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore { pool };

        assert!(
            store
                .set_enabled("startup_backfill.proxy_usage", true)
                .await
                .expect("enable scheduled task")
        );
        store
            .request_run("startup_backfill.proxy_usage")
            .await
            .expect("queue enabled task run");
        assert!(
            store
                .set_enabled("startup_backfill.proxy_usage", false)
                .await
                .expect("disable scheduled task")
        );

        assert!(store.claim_requested_run().await.unwrap().is_none());
        let row = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status,summary FROM managed_task_runs WHERE task_key='startup_backfill.proxy_usage'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load retired requested run");
        assert_eq!(row.0, "failed");
        assert_eq!(row.1.as_deref(), Some("任务已停用，未执行"));
    }

    #[tokio::test]
    async fn applies_initial_task_defaults_only_once() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "UPDATE managed_tasks SET enabled=1 WHERE task_key IN ('startup_backfill','raw_compression')",
        )
        .execute(&pool)
        .await
        .expect("simulate pre-default task controls");
        sqlx::query(
            "INSERT INTO startup_backfill_progress (task_name,enabled) VALUES ('proxy_usage_tokens_v1',1)",
        )
        .execute(&pool)
        .await
        .expect("insert backfill control row");

        let store = MaintenanceStore { pool };
        assert!(store.apply_initial_task_defaults().await.unwrap());
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='startup_backfill'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='raw_compression'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='startup_backfill.proxy_usage'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT enabled FROM startup_backfill_progress WHERE task_name='proxy_usage_tokens_v1'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap(),
            1
        );

        sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key='raw_compression'")
            .execute(&store.pool)
            .await
            .expect("simulate operator enablement");
        assert!(!store.apply_initial_task_defaults().await.unwrap());
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='raw_compression'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
    }

    #[tokio::test]
    async fn retention_default_schedule_is_observable_and_overrideable() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore { pool };

        assert!(store.apply_initial_task_defaults().await.unwrap());
        let task = store
            .detail("retention_archive")
            .await
            .unwrap()
            .expect("retention task detail");
        let schedule = task
            .task
            .effective_schedule
            .expect("default schedule should be published");
        assert_eq!(schedule.source, "default");
        assert_eq!(schedule.interval_secs, Some(3_600));
        assert!(schedule.next_trigger_at.is_some());

        assert!(
            store
                .set_schedule("retention_archive", Some(1_800), None)
                .await
                .unwrap()
        );
        let task = store
            .detail("retention_archive")
            .await
            .unwrap()
            .expect("overridden retention task detail");
        assert_eq!(
            task.task
                .effective_schedule
                .expect("override schedule should be published")
                .source,
            "override"
        );
    }

    #[tokio::test]
    async fn reconciles_legacy_backfill_enablement_after_defaults_marker_exists() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "INSERT INTO maintenance_metadata (key,value,updated_at) VALUES ('managed_task_defaults_v1','applied','2026-10-01T00:00:00.000Z')",
        )
        .execute(&pool)
        .await
        .expect("seed existing defaults marker");
        sqlx::query(
            "INSERT INTO startup_backfill_progress (task_name,enabled) VALUES ('proxy_usage_tokens_v1',1)",
        )
        .execute(&pool)
        .await
        .expect("insert legacy enabled backfill row");

        let store = MaintenanceStore { pool };
        assert!(store.apply_initial_task_defaults().await.unwrap());
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='startup_backfill.proxy_usage'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
        assert!(!store.apply_initial_task_defaults().await.unwrap());
    }

    #[tokio::test]
    async fn migrates_versioned_backfill_keys_and_preserves_interrupted_history() {
        let main_pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect legacy main test pool");
        sqlx::query(
            "CREATE TABLE system_task_runs (id INTEGER PRIMARY KEY, task_kind TEXT NOT NULL, trigger_kind TEXT NOT NULL, status TEXT NOT NULL, summary TEXT, detail TEXT, started_at TEXT NOT NULL, finished_at TEXT, duration_ms INTEGER)",
        )
        .execute(&main_pool)
        .await
        .expect("create legacy task history table");
        sqlx::query(
            "CREATE TABLE startup_backfill_progress (task_name TEXT PRIMARY KEY, cursor_id INTEGER NOT NULL, next_run_after TEXT, zero_update_streak INTEGER NOT NULL, last_started_at TEXT, last_finished_at TEXT, last_scanned INTEGER NOT NULL, last_updated INTEGER NOT NULL, last_status TEXT NOT NULL, suspension_reason TEXT, next_probe_at TEXT, wake_generation INTEGER NOT NULL, enabled INTEGER NOT NULL)",
        )
        .execute(&main_pool)
        .await
        .expect("create legacy backfill table");
        for id in 1..=3 {
            sqlx::query(
                "INSERT INTO system_task_runs (id,task_kind,trigger_kind,status,started_at) VALUES (?,?,?,?,?)",
            )
            .bind(id)
            .bind("retention_archive")
            .bind("startup")
            .bind("running")
            .bind(format!("2026-09-30T00:00:0{id}.000Z"))
            .execute(&main_pool)
            .await
            .expect("insert interrupted legacy run");
        }
        for (id, task_name) in [
            (4, "proxy_usage_tokens_v1"),
            (5, "proxy_cost_v1:catalog-version"),
        ] {
            sqlx::query(
                "INSERT INTO startup_backfill_progress (task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(task_name)
            .bind(id)
            .bind(Option::<String>::None)
            .bind(0_i64)
            .bind(Option::<String>::None)
            .bind(Some("2026-09-30T00:00:00.000Z"))
            .bind(10_i64)
            .bind(id)
            .bind("ok")
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(0_i64)
            .bind(1_i64)
            .execute(&main_pool)
            .await
            .expect("insert versioned legacy progress");
        }

        let maintenance_pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&maintenance_pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&maintenance_pool)
            .await
            .expect("seed maintenance task registry");
        let store = MaintenanceStore {
            pool: maintenance_pool,
        };

        store
            .migrate_legacy_state(&main_pool)
            .await
            .expect("migrate legacy state");

        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM managed_task_runs")
                .fetch_one(&store.pool)
                .await
                .expect("count migrated runs"),
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM managed_task_runs WHERE status IN ('running','requested')",
            )
            .fetch_one(&store.pool)
            .await
            .expect("count active migrated runs"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM managed_task_progress")
                .fetch_one(&store.pool)
                .await
                .expect("count managed progress snapshots"),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM managed_task_progress WHERE task_key IN ('startup_backfill.proxy_usage','startup_backfill.proxy_cost')",
            )
            .fetch_one(&store.pool)
            .await
            .expect("count canonical progress snapshots"),
            2
        );
        let migrated_run = sqlx::query_as::<_, (String, Option<String>, Option<i64>)>(
            "SELECT status,finished_at,duration_ms FROM managed_task_runs ORDER BY id LIMIT 1",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load migrated interrupted run");
        assert_eq!(migrated_run.0, "failed");
        assert!(migrated_run.1.is_some());
        assert_eq!(migrated_run.2, Some(0));
    }
}
