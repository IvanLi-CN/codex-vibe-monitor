use crate::{AppConfig, OnceCell, Result, Utc, anyhow, format_utc_iso_millis};
use serde::Serialize;
use sqlx::{
    FromRow, Pool, Sqlite,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::PathBuf, str::FromStr, time::Duration};

const MIN_INTERVAL_SECS: i64 = 60;

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
    pub(crate) started_at: String,
    pub(crate) finished_at: Option<String>,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) status: String,
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
}

pub(crate) const MANAGED_TASKS: &[(&str, &str, &str, &str, bool)] = &[
    (
        "retention_archive",
        "Retention archive",
        "Retention and archive maintenance",
        "interval",
        false,
    ),
    (
        "upstream_account_maintenance",
        "Upstream account maintenance",
        "Account sync and routing maintenance",
        "interval",
        false,
    ),
    (
        "forward_proxy_subscription_refresh",
        "Forward proxy subscription refresh",
        "Refresh configured subscription proxies",
        "event",
        false,
    ),
    (
        "pool_orphan_recovery",
        "Pool orphan recovery",
        "Recover stale pool rows",
        "interval",
        false,
    ),
    (
        "startup_hourly_rollup_bootstrap",
        "Hourly rollup bootstrap",
        "Repair missing startup rollups",
        "startup",
        false,
    ),
    (
        "system_status_snapshot",
        "System status snapshot",
        "Refresh system status read model",
        "interval",
        false,
    ),
    (
        "invocation_timeline_snapshot",
        "Invocation timeline snapshot",
        "Clean and refresh timeline snapshot",
        "interval",
        false,
    ),
    (
        "summary_snapshot",
        "Summary snapshot",
        "Publish summary projection snapshot",
        "event",
        false,
    ),
    (
        "summary_coverage_recovery",
        "Summary coverage recovery",
        "Repair summary coverage gaps",
        "interval",
        false,
    ),
    (
        "dashboard_runtime_projection_reconcile",
        "Dashboard projection reconcile",
        "Reconcile dashboard runtime projection",
        "event",
        false,
    ),
    (
        "long_term_projection",
        "Long term projection",
        "Refresh long term statistics",
        "interval",
        false,
    ),
    (
        "timeseries_minute_projection",
        "Timeseries minute projection",
        "Refresh minute projections",
        "interval",
        false,
    ),
    (
        "raw_payload_metrics_inventory",
        "Raw payload inventory",
        "Inventory raw payload metrics",
        "interval",
        false,
    ),
    (
        "prompt_cache_materialization",
        "Prompt cache materialization",
        "Materialize prompt cache conversations",
        "event",
        false,
    ),
    (
        "startup_backfill",
        "Startup backfill",
        "Startup backfill parent task",
        "startup",
        false,
    ),
    (
        "raw_compression",
        "Raw compression",
        "Compress cold raw payloads",
        "manual",
        true,
    ),
    (
        "archive_upstream_activity_manifest",
        "Archive activity manifest",
        "Refresh archive activity manifests",
        "manual",
        true,
    ),
    (
        "materialize_historical_rollups",
        "Historical rollups",
        "Materialize historical rollups",
        "manual",
        true,
    ),
    (
        "verify_archive_storage",
        "Verify archive storage",
        "Verify archive files and manifests",
        "manual",
        true,
    ),
    (
        "prune_archive_batches",
        "Prune archive batches",
        "Prune safe archive batches",
        "manual",
        true,
    ),
    (
        "prune_legacy_archive_batches",
        "Prune legacy archive batches",
        "Prune legacy archive batches",
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
    if fields
        .iter()
        .any(|field| field.contains(';') || field.contains('\n'))
    {
        return Err(anyhow!("cron expression contains an invalid character"));
    }
    Ok(())
}

async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    for statement in r#"
        CREATE TABLE IF NOT EXISTS managed_tasks (
          task_key TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL,
          trigger_mode TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
          interval_secs INTEGER, cron_expr TEXT, is_manual INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS managed_task_progress (
          task_key TEXT PRIMARY KEY REFERENCES managed_tasks(task_key) ON DELETE CASCADE,
          total INTEGER, completed INTEGER, phase TEXT, checkpoint TEXT, eta_seconds INTEGER,
          updated_at TEXT, freshness TEXT NOT NULL DEFAULT 'fresh'
        );
        CREATE TABLE IF NOT EXISTS managed_task_runs (
          id INTEGER PRIMARY KEY AUTOINCREMENT, legacy_id INTEGER UNIQUE, task_key TEXT NOT NULL,
          started_at TEXT NOT NULL, finished_at TEXT, duration_ms INTEGER,
          status TEXT NOT NULL, processed_count INTEGER, updated_count INTEGER,
          error_detail TEXT
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
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_task_runs_legacy_id ON managed_task_runs(legacy_id) WHERE legacy_id IS NOT NULL")
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
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,is_manual,updated_at) VALUES (?,?,?,?,?,?) ON CONFLICT(task_key) DO NOTHING")
            .bind(format!("startup_backfill.{key}")).bind(*key).bind("Startup backfill child task").bind("event").bind(0_i64).bind(&now).execute(pool).await?;
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

impl MaintenanceStore {
    pub(crate) async fn request_run(&self, task_key: &str) -> Result<i64> {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM managed_tasks WHERE task_key=?")
                .bind(task_key)
                .fetch_optional(&self.pool)
                .await?;
        if exists.is_none() {
            return Err(anyhow!("managed task not found"));
        }
        let active: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM managed_task_runs WHERE task_key=? AND status IN ('running','requested') ORDER BY id DESC LIMIT 1",
        )
        .bind(task_key)
        .fetch_optional(&self.pool)
        .await?;
        if active.is_some() {
            return Err(anyhow!("task already has an active run"));
        }
        Ok(sqlx::query_scalar("INSERT INTO managed_task_runs (task_key,started_at,status,error_detail) VALUES (?,?,?,?) RETURNING id")
            .bind(task_key)
            .bind(format_utc_iso_millis(Utc::now()))
            .bind("requested")
            .bind(Option::<String>::None)
            .fetch_one(&self.pool)
            .await?)
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
            let error_detail = detail.or(summary);
            sqlx::query(
                "INSERT OR IGNORE INTO managed_task_runs (legacy_id,task_key,started_at,finished_at,duration_ms,status,error_detail) VALUES (?,?,?,?,?,?,?)",
            )
            .bind(id)
            .bind(task_key)
            .bind(started_at)
            .bind(finished_at)
            .bind(duration_ms)
            .bind(format!("{status}:{trigger_kind}"))
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
            _zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status,
            _suspension_reason,
            _next_probe_at,
            _wake_generation,
            enabled,
        ) in progress
        {
            let task_key = format!("startup_backfill.{task_name}");
            let updated_at = last_finished_at.or(last_started_at).or(next_run_after);
            let freshness = if enabled == 0 { "stale" } else { "fresh" };
            sqlx::query(
                "INSERT INTO managed_task_progress (task_key,total,completed,phase,checkpoint,updated_at,freshness) VALUES (?,?,?,?,?,?,?) ON CONFLICT(task_key) DO UPDATE SET completed=excluded.completed,phase=excluded.phase,checkpoint=excluded.checkpoint,updated_at=excluded.updated_at,freshness=excluded.freshness",
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

    pub(crate) async fn begin_run(&self, task_key: &str, started_at: &str) -> Result<i64> {
        Ok(sqlx::query_scalar("INSERT INTO managed_task_runs (task_key,started_at,status) VALUES (?,?,?) RETURNING id")
            .bind(task_key).bind(started_at).bind("running").fetch_one(&self.pool).await?)
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
        sqlx::query("UPDATE managed_task_runs SET status=?,finished_at=?,duration_ms=?,error_detail=COALESCE(?,?) WHERE id=?")
            .bind(status).bind(finished_at).bind(duration_ms).bind(detail).bind(summary).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub(crate) async fn list_tasks(&self) -> Result<Vec<ManagedTask>> {
        Ok(sqlx::query_as("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,is_manual FROM managed_tasks ORDER BY task_key")
        .fetch_all(&self.pool).await?)
    }

    pub(crate) async fn detail(&self, task_key: &str) -> Result<Option<ManagedTaskDetail>> {
        let task = sqlx::query_as::<_, ManagedTask>("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,is_manual FROM managed_tasks WHERE task_key=?")
        .bind(task_key).fetch_optional(&self.pool).await?;
        let Some(task) = task else {
            return Ok(None);
        };
        let progress = sqlx::query_as::<_, TaskProgress>("SELECT total,completed,phase,checkpoint,eta_seconds,updated_at,freshness FROM managed_task_progress WHERE task_key=?")
        .bind(task_key).fetch_optional(&self.pool).await?;
        let recent_runs = sqlx::query_as::<_, TaskRun>("SELECT id,started_at,finished_at,duration_ms,status,processed_count,updated_count,error_detail FROM managed_task_runs WHERE task_key=? ORDER BY started_at DESC LIMIT 10")
        .bind(task_key).fetch_all(&self.pool).await?;
        Ok(Some(ManagedTaskDetail {
            task,
            progress,
            recent_runs,
        }))
    }

    pub(crate) async fn set_enabled(&self, task_key: &str, enabled: bool) -> Result<bool> {
        let result =
            sqlx::query("UPDATE managed_tasks SET enabled=?, updated_at=? WHERE task_key=?")
                .bind(enabled as i64)
                .bind(format_utc_iso_millis(Utc::now()))
                .bind(task_key)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
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
        Self::validate_interval(interval_secs).await?;
        validate_cron_expr(cron_expr)?;
        let result = sqlx::query("UPDATE managed_tasks SET interval_secs=?, cron_expr=?, updated_at=? WHERE task_key=? AND is_manual=0")
        .bind(interval_secs).bind(cron_expr).bind(format_utc_iso_millis(Utc::now())).bind(task_key).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }
}

pub(crate) fn path(config: &AppConfig) -> PathBuf {
    config.maintenance_database_path()
}
