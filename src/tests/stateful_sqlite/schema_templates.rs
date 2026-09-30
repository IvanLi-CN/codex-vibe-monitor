use super::*;
use libsqlite3_sys::{
    SQLITE_DONE, SQLITE_OK, sqlite3_backup_finish, sqlite3_backup_init, sqlite3_backup_step,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub(crate) const STATEFUL_SCHEMA_TEMPLATE_PATH_ENV: &str =
    "CODEX_VIBE_MONITOR_STATEFUL_SCHEMA_TEMPLATE_PATH";
pub(crate) const LIGHTWEIGHT_SCHEMA_TEMPLATE_PATH_ENV: &str =
    "CODEX_VIBE_MONITOR_LIGHTWEIGHT_SCHEMA_TEMPLATE_PATH";
pub(crate) const ARCHIVE_SCHEMA_TEMPLATE_PATH_ENV: &str =
    "CODEX_VIBE_MONITOR_ARCHIVE_SCHEMA_TEMPLATE_PATH";

pub(crate) fn current_profile_schema_template_path() -> Option<PathBuf> {
    std::env::var_os(STATEFUL_SCHEMA_TEMPLATE_PATH_ENV)
        .or_else(|| std::env::var_os(LIGHTWEIGHT_SCHEMA_TEMPLATE_PATH_ENV))
        .or_else(|| std::env::var_os(ARCHIVE_SCHEMA_TEMPLATE_PATH_ENV))
        .map(PathBuf::from)
}

fn current_test_state_schema_template_path() -> Option<PathBuf> {
    std::env::var_os(STATEFUL_SCHEMA_TEMPLATE_PATH_ENV)
        .or_else(|| std::env::var_os(LIGHTWEIGHT_SCHEMA_TEMPLATE_PATH_ENV))
        .map(PathBuf::from)
}

pub(crate) async fn write_current_schema_template(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "create current-schema template directory {}",
                parent.display()
            )
        })?;
    }
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("remove stale current-schema template {}", path.display()))?;
    }

    let options = SqliteConnectOptions::from_str(&test_sqlite_url_for_path(path))
        .context("build current-schema template sqlite options")?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("open current-schema template {}", path.display()))?;
    ensure_schema(&pool)
        .await
        .context("initialize current-schema template")?;
    pool.close().await;
    Ok(())
}

pub(crate) async fn restore_test_state_schema_template(pool: &SqlitePool) -> anyhow::Result<()> {
    let Some(template_path) = current_test_state_schema_template_path() else {
        return ensure_schema(pool).await;
    };
    if !template_path.is_file() {
        anyhow::bail!(
            "current-profile schema template does not exist: {}",
            template_path.display()
        );
    }
    restore_current_schema_template_from_path(pool, &template_path).await
}

pub(crate) async fn restore_current_schema_template_from_path(
    pool: &SqlitePool,
    template_path: &Path,
) -> anyhow::Result<()> {
    let options = SqliteConnectOptions::from_str(&test_sqlite_url_for_path(template_path))
        .context("build current-schema template reader options")?
        .read_only(true)
        .create_if_missing(false);
    let mut source = SqliteConnection::connect_with(&options)
        .await
        .with_context(|| format!("open current-schema template {}", template_path.display()))?;
    let mut destination = pool.acquire().await.context("acquire test sqlite")?;
    let mut destination_handle = destination
        .lock_handle()
        .await
        .context("lock test sqlite handle")?;
    let mut source_handle = source
        .lock_handle()
        .await
        .context("lock current-schema template handle")?;

    // Copy the real schema once instead of replaying DDL for each test state.
    let backup = unsafe {
        sqlite3_backup_init(
            destination_handle.as_raw_handle().as_ptr(),
            c"main".as_ptr(),
            source_handle.as_raw_handle().as_ptr(),
            c"main".as_ptr(),
        )
    };
    if backup.is_null() {
        anyhow::bail!("start current-schema SQLite backup");
    }
    let step_code = unsafe { sqlite3_backup_step(backup, -1) };
    let finish_code = unsafe { sqlite3_backup_finish(backup) };
    if step_code != SQLITE_DONE || finish_code != SQLITE_OK {
        anyhow::bail!(
            "copy current-schema SQLite backup failed: step={step_code}, finish={finish_code}"
        );
    }

    drop(source_handle);
    drop(destination_handle);
    drop(destination);
    source
        .close()
        .await
        .context("close current-schema template")?;
    Ok(())
}
