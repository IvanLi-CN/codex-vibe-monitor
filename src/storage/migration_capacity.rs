//! Bounded source-volume and capture-capacity observations for online migration.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MigrationCapacitySample {
    pub(crate) source_free_bytes: Option<u64>,
    pub(crate) source_volume_bytes: Option<u64>,
    pub(crate) capture_bytes: u64,
    pub(crate) capture_limit_bytes: u64,
    pub(crate) capture_capacity_held: bool,
    pub(crate) continuity_lost: bool,
    pub(crate) baseline_required: bool,
    pub(crate) source_volume_is_memory: bool,
}

impl MigrationCapacitySample {
    pub(crate) fn unavailable(capture_limit_bytes: u64) -> Self {
        Self {
            source_free_bytes: None,
            source_volume_bytes: None,
            capture_bytes: 0,
            capture_limit_bytes: capture_limit_bytes.max(1),
            capture_capacity_held: false,
            continuity_lost: false,
            baseline_required: false,
            source_volume_is_memory: false,
        }
    }

    pub(crate) fn hold_reason(self, min_source_free_bytes: u64) -> Option<&'static str> {
        if !self.source_volume_is_memory && self.source_free_bytes.is_none() {
            return Some("capacity_source_unavailable");
        }
        if self
            .source_free_bytes
            .is_some_and(|available| available < min_source_free_bytes)
        {
            return Some("capacity_source_free");
        }
        if self.continuity_lost {
            return Some("capture_continuity_lost");
        }
        if self.capture_capacity_held || self.capture_bytes >= self.capture_limit_bytes {
            return Some("capacity_capture");
        }
        None
    }
}

pub(crate) async fn sample_sqlite_capacity(
    pool: &SqlitePool,
    capture_state_table_name: &str,
    migration_id: &str,
    source_identity: &str,
    default_capture_limit_bytes: u64,
) -> Result<MigrationCapacitySample> {
    let database = inspect_main_database(pool).await?;
    let (source_free_bytes, source_volume_bytes, source_volume_is_memory) = match database.path {
        Some(path) => (
            filesystem_available_bytes(path.parent().unwrap_or_else(|| Path::new("."))),
            Some(database_volume_bytes(&path, database.page_bytes)),
            false,
        ),
        None => (Some(u64::MAX), None, true),
    };

    let mut sample = MigrationCapacitySample {
        source_free_bytes,
        source_volume_bytes,
        capture_bytes: 0,
        capture_limit_bytes: default_capture_limit_bytes.max(1),
        capture_capacity_held: false,
        continuity_lost: false,
        baseline_required: false,
        source_volume_is_memory,
    };

    if !table_exists(pool, capture_state_table_name).await? {
        return Ok(sample);
    }

    let state_table = quote_identifier(capture_state_table_name)?;
    let row = sqlx::query(&format!(
        "SELECT captured_bytes, max_capture_bytes, capture_capacity_held,
                continuity_lost, baseline_required
           FROM {state_table}
          WHERE migration_id = ? AND source_identity = ?"
    ))
    .bind(migration_id)
    .bind(source_identity)
    .fetch_optional(pool)
    .await
    .context("read SQLite migration capacity state")?;

    if let Some(row) = row {
        sample.capture_bytes = u64::try_from(row.try_get::<i64, _>("captured_bytes")?)
            .context("SQLite migration capture bytes are negative")?;
        sample.capture_limit_bytes = u64::try_from(row.try_get::<i64, _>("max_capture_bytes")?)
            .context("SQLite migration capture capacity is negative")?
            .max(1);
        sample.capture_capacity_held = row.try_get::<i64, _>("capture_capacity_held")? != 0;
        sample.continuity_lost = row.try_get::<i64, _>("continuity_lost")? != 0;
        sample.baseline_required = row.try_get::<i64, _>("baseline_required")? != 0;
    }
    Ok(sample)
}

#[derive(Debug)]
struct MainDatabase {
    path: Option<PathBuf>,
    page_bytes: Option<u64>,
}

async fn inspect_main_database(pool: &SqlitePool) -> Result<MainDatabase> {
    let page_count = sqlx::query_scalar::<_, i64>("PRAGMA page_count")
        .fetch_one(pool)
        .await
        .context("read SQLite source page count")?;
    let page_size = sqlx::query_scalar::<_, i64>("PRAGMA page_size")
        .fetch_one(pool)
        .await
        .context("read SQLite source page size")?;
    let page_bytes = (page_count >= 0 && page_size >= 0)
        .then(|| {
            u64::try_from(page_count)
                .ok()?
                .checked_mul(u64::try_from(page_size).ok()?)
        })
        .flatten();

    let row = sqlx::query("PRAGMA database_list")
        .fetch_all(pool)
        .await
        .context("inspect SQLite source database list")?
        .into_iter()
        .find(|row| row.try_get::<String, _>("name").ok().as_deref() == Some("main"));
    let path = row
        .and_then(|row| row.try_get::<String, _>("file").ok())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    Ok(MainDatabase { path, page_bytes })
}

async fn table_exists(pool: &SqlitePool, table_name: &str) -> Result<bool> {
    sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?
         )",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await
    .context("inspect SQLite migration capacity state table")
    .map(|value| value != 0)
}

fn quote_identifier(identifier: &str) -> Result<String> {
    let mut characters = identifier.chars();
    let Some(first) = characters.next() else {
        return Err(anyhow!("SQLite migration capacity table name is empty"));
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || characters.any(|character| !(character == '_' || character.is_ascii_alphanumeric()))
    {
        return Err(anyhow!(
            "SQLite migration capacity table name is not a safe identifier"
        ));
    }
    Ok(format!("\"{identifier}\""))
}

fn database_volume_bytes(path: &Path, page_bytes: Option<u64>) -> u64 {
    let main_bytes = page_bytes.or_else(|| file_size(path)).unwrap_or_default();
    main_bytes
        .saturating_add(file_size(&PathBuf::from(format!("{}-wal", path.display()))).unwrap_or(0))
        .saturating_add(
            file_size(&PathBuf::from(format!("{}-journal", path.display()))).unwrap_or(0),
        )
        .saturating_add(file_size(&PathBuf::from(format!("{}-shm", path.display()))).unwrap_or(0))
}

fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.len())
}

fn filesystem_available_bytes(root: &Path) -> Option<u64> {
    crate::filesystem_available_bytes(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migration::{DEFAULT_MAX_CAPTURE_BYTES, DEFAULT_MIN_SOURCE_FREE_BYTES};

    #[test]
    fn capacity_hold_reason_is_bounded_and_non_sensitive() {
        let mut sample = MigrationCapacitySample {
            source_free_bytes: Some(DEFAULT_MIN_SOURCE_FREE_BYTES - 1),
            source_volume_bytes: Some(1024),
            capture_bytes: 0,
            capture_limit_bytes: DEFAULT_MAX_CAPTURE_BYTES,
            capture_capacity_held: false,
            continuity_lost: false,
            baseline_required: false,
            source_volume_is_memory: false,
        };
        assert_eq!(
            sample.hold_reason(DEFAULT_MIN_SOURCE_FREE_BYTES),
            Some("capacity_source_free")
        );
        sample.source_free_bytes = Some(DEFAULT_MIN_SOURCE_FREE_BYTES);
        sample.capture_capacity_held = true;
        assert_eq!(
            sample.hold_reason(DEFAULT_MIN_SOURCE_FREE_BYTES),
            Some("capacity_capture")
        );
        sample.capture_capacity_held = false;
        sample.continuity_lost = true;
        assert_eq!(
            sample.hold_reason(DEFAULT_MIN_SOURCE_FREE_BYTES),
            Some("capture_continuity_lost")
        );
        assert!(sample.source_volume_bytes.is_some());
    }
}
