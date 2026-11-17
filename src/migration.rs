use super::*;
use crate::primary_db::PrimaryDatabase;
mod direct;
use direct::{reset_direct_import, verify_direct_payloads};

const MIGRATION_SCHEMA_SQL: &str = r#"
CREATE SCHEMA IF NOT EXISTS cvm_core;
CREATE SCHEMA IF NOT EXISTS cvm_activity;
CREATE SCHEMA IF NOT EXISTS cvm_stats;
CREATE SCHEMA IF NOT EXISTS cvm_archive;
CREATE SCHEMA IF NOT EXISTS cvm_assets;
CREATE SCHEMA IF NOT EXISTS cvm_migration;
CREATE TABLE IF NOT EXISTS cvm_migration.runs (
    run_id TEXT PRIMARY KEY,
    target_profile TEXT NOT NULL,
    source_root TEXT NOT NULL,
    source_manifest_sha256 TEXT,
    state TEXT NOT NULL CHECK (state IN ('prepared','seeding','seeded','source_stopped','catching_up','verified','finalized','paused','failed')),
    resume_state TEXT CHECK (resume_state IN ('prepared','source_stopped')),
    last_error_code TEXT,
    needs_reset BOOLEAN NOT NULL DEFAULT FALSE,
    snapshot_sha256 TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS cvm_migration.objects (
    run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE,
    object_name TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    source_rows BIGINT NOT NULL DEFAULT 0,
    imported_rows BIGINT NOT NULL DEFAULT 0,
    checkpoint TEXT,
    checksum TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, object_name)
);
CREATE TABLE IF NOT EXISTS cvm_migration.object_columns (
    run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE,
    object_name TEXT NOT NULL,
    ordinal BIGINT NOT NULL,
    column_name TEXT NOT NULL,
    declared_type TEXT NOT NULL DEFAULT '',
    primary_key_position BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, object_name, ordinal)
);
CREATE TABLE IF NOT EXISTS cvm_migration.object_foreign_keys (
    run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE,
    object_name TEXT NOT NULL,
    constraint_id BIGINT NOT NULL,
    sequence_no BIGINT NOT NULL,
    column_name TEXT NOT NULL,
    referenced_object_name TEXT NOT NULL,
    referenced_column_name TEXT NOT NULL,
    on_update TEXT NOT NULL,
    on_delete TEXT NOT NULL,
    PRIMARY KEY (run_id, object_name, constraint_id, sequence_no)
);
CREATE TABLE IF NOT EXISTS cvm_migration.object_unique_indexes (
    run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE,
    object_name TEXT NOT NULL,
    index_name TEXT NOT NULL,
    sequence_no BIGINT NOT NULL,
    column_name TEXT NOT NULL,
    predicate TEXT,
    PRIMARY KEY (run_id, object_name, index_name, sequence_no)
);
CREATE TABLE IF NOT EXISTS cvm_migration.object_targets (
    run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE,
    object_name TEXT NOT NULL,
    target_schema TEXT NOT NULL,
    target_table TEXT NOT NULL,
    PRIMARY KEY (run_id, object_name)
);
CREATE TABLE IF NOT EXISTS cvm_migration.assets (
    run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    sha256 TEXT NOT NULL,
    asset_kind TEXT NOT NULL,
    PRIMARY KEY (run_id, relative_path)
);
CREATE TABLE IF NOT EXISTS cvm_assets.asset_index (
    run_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    sha256 TEXT NOT NULL,
    asset_kind TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, relative_path)
);
CREATE TABLE IF NOT EXISTS cvm_migration.batches (
    run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE,
    object_name TEXT NOT NULL,
    batch_no BIGINT NOT NULL,
    first_key TEXT,
    last_key TEXT,
    row_count BIGINT NOT NULL,
    checksum TEXT NOT NULL,
    source_cursor TEXT,
    committed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, object_name, batch_no)
);
"#;
pub(crate) const MIGRATION_SCHEMA_VERSION: i64 = 11;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceManifest {
    format_version: u32,
    source_root: String,
    tables: Vec<SourceTable>,
    #[serde(default)]
    foreign_keys: Vec<SourceForeignKey>,
    #[serde(default)]
    unique_indexes: Vec<SourceUniqueIndex>,
    #[serde(default)]
    assets: Vec<SourceAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SourceForeignKey {
    table: String,
    constraint_id: i64,
    sequence: i64,
    column: String,
    referenced_table: String,
    referenced_column: String,
    on_update: String,
    on_delete: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SourceUniqueIndex {
    table: String,
    name: String,
    columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    predicate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceAsset {
    relative_path: String,
    size_bytes: u64,
    sha256: String,
    asset_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceTable {
    name: String,
    columns: Vec<SourceColumn>,
    primary_key: Vec<String>,
    row_count: i64,
    kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceColumn {
    name: String,
    declared_type: String,
}

async fn target_pool(config: &AppConfig) -> Result<sqlx::PgPool> {
    if config.primary_database_backend != PrimaryDatabaseBackend::Postgres {
        bail!("migration commands require PRIMARY_DATABASE_BACKEND=postgres");
    }
    let url = config
        .postgres_primary_migration_url
        .as_ref()
        .context("POSTGRES_PRIMARY_MIGRATION_URL is required for migration commands")?;
    let database =
        PrimaryDatabase::connect_url(url, config.postgres_primary_migration_max_connections)
            .await?;
    Ok(database.pool().clone())
}

async fn ensure_target_schema(config: &AppConfig, pool: &sqlx::PgPool) -> Result<()> {
    let primary_database = PrimaryDatabase::connect(config).await?;
    let ddl_database = PrimaryDatabase::from_pool(pool.clone());
    let primary_database_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(primary_database.pool())
        .await
        .context("failed to identify PostgreSQL primary database")?;
    let ddl_database_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(ddl_database.pool())
        .await
        .context("failed to identify PostgreSQL migration database")?;
    if primary_database_name != ddl_database_name {
        bail!(
            "PostgreSQL primary and migration URLs must point to the same database: primary={primary_database_name}, migration={ddl_database_name}"
        );
    }
    ddl_database.ensure_schema().await?;
    let ddl_pool = ddl_database.pool();
    let legacy: bool =
        sqlx::query_scalar("SELECT to_regclass('cvm_migration.imported_rows') IS NOT NULL")
            .fetch_one(ddl_pool)
            .await?;
    if legacy {
        bail!(
            "legacy JSON-copy migration layout detected; preserve this database and import the source into a fresh PostgreSQL database with the direct importer"
        );
    }
    sqlx::raw_sql(MIGRATION_SCHEMA_SQL)
        .execute(ddl_pool)
        .await
        .context("failed to ensure PostgreSQL migration schema")?;
    let checksum = format!("{:x}", Sha256::digest(MIGRATION_SCHEMA_SQL.as_bytes()));
    let existing_checksum: Option<String> = sqlx::query_scalar(
        "SELECT checksum FROM cvm_migration.schema_migrations WHERE component='migration-ledger' AND version=$1",
    )
    .bind(MIGRATION_SCHEMA_VERSION)
    .fetch_optional(ddl_pool)
    .await
    .context("failed to read migration schema checksum")?;
    if let Some(existing_checksum) = existing_checksum.as_ref() {
        if existing_checksum != &checksum {
            bail!(
                "migration schema checksum mismatch; rebuild the target schema before continuing"
            );
        }
    } else {
        for statement in [
            "ALTER TABLE cvm_migration.runs ADD COLUMN IF NOT EXISTS target_profile TEXT",
            "ALTER TABLE cvm_migration.runs ADD COLUMN IF NOT EXISTS source_root TEXT",
            "ALTER TABLE cvm_migration.runs ADD COLUMN IF NOT EXISTS resume_state TEXT",
            "UPDATE cvm_migration.runs SET target_profile = 'default' WHERE target_profile IS NULL",
            "ALTER TABLE cvm_migration.runs ALTER COLUMN target_profile SET NOT NULL",
            "ALTER TABLE cvm_migration.object_columns ADD COLUMN IF NOT EXISTS declared_type TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE cvm_migration.batches ADD COLUMN IF NOT EXISTS source_cursor TEXT",
            "CREATE TABLE IF NOT EXISTS cvm_migration.object_foreign_keys (run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE, object_name TEXT NOT NULL, constraint_id BIGINT, sequence_no BIGINT, column_name TEXT NOT NULL, referenced_object_name TEXT NOT NULL, referenced_column_name TEXT NOT NULL, on_update TEXT NOT NULL, on_delete TEXT NOT NULL, PRIMARY KEY (run_id, object_name, column_name, referenced_object_name, referenced_column_name))",
            "ALTER TABLE cvm_migration.object_foreign_keys ADD COLUMN IF NOT EXISTS constraint_id BIGINT",
            "ALTER TABLE cvm_migration.object_foreign_keys ADD COLUMN IF NOT EXISTS sequence_no BIGINT",
            "CREATE TABLE IF NOT EXISTS cvm_migration.object_unique_indexes (run_id TEXT NOT NULL REFERENCES cvm_migration.runs(run_id) ON DELETE CASCADE, object_name TEXT NOT NULL, index_name TEXT NOT NULL, sequence_no BIGINT, column_name TEXT NOT NULL, PRIMARY KEY (run_id, object_name, index_name, sequence_no))",
            "ALTER TABLE cvm_migration.runs ADD COLUMN IF NOT EXISTS needs_reset BOOLEAN NOT NULL DEFAULT FALSE",
            "ALTER TABLE cvm_migration.runs ADD COLUMN IF NOT EXISTS snapshot_sha256 TEXT",
            "ALTER TABLE cvm_migration.object_unique_indexes ADD COLUMN IF NOT EXISTS predicate TEXT",
        ] {
            sqlx::query(statement)
                .execute(ddl_pool)
                .await
                .with_context(|| {
                    format!("failed to apply migration schema upgrade: {statement}")
                })?;
        }
        sqlx::query(
            "INSERT INTO cvm_migration.schema_migrations(component,version,checksum) VALUES('migration-ledger',$1,$2)",
        )
        .bind(MIGRATION_SCHEMA_VERSION)
        .bind(&checksum)
        .execute(ddl_pool)
        .await
        .context("failed to record migration schema checksum")?;
    }
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(ddl_pool)
        .await?;
    if !(180000..190000).contains(&version) {
        bail!("unsupported PostgreSQL major version; expected 18.x");
    }
    for schema in [
        "cvm_core",
        "cvm_activity",
        "cvm_stats",
        "cvm_archive",
        "cvm_assets",
        "cvm_migration",
    ] {
        let (can_use_schema, can_create_schema): (bool, bool) = sqlx::query_as(
            "SELECT has_schema_privilege(current_user, $1, 'USAGE'), has_schema_privilege(current_user, $1, 'CREATE')",
        )
        .bind(schema)
        .fetch_one(ddl_pool)
        .await
        .with_context(|| format!("failed to inspect PostgreSQL migration role privileges on {schema}"))?;
        if !can_use_schema || !can_create_schema {
            bail!("migration role must have USAGE and CREATE privileges on {schema}");
        }
    }
    Ok(())
}

async fn source_pool(root: &Path) -> Result<sqlx::SqlitePool> {
    let database = source_database_path(root);
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&database)
        .read_only(true)
        .create_if_missing(false);
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| {
            format!(
                "failed to open source SQLite database {}",
                database.display()
            )
        })
}

fn source_database_path(root: &Path) -> PathBuf {
    if root.is_file() {
        root.to_path_buf()
    } else {
        let configured =
            env::var("DATABASE_PATH").unwrap_or_else(|_| "codex_vibe_monitor.db".into());
        root.join(configured)
    }
}

fn manifest_checksum(manifest: &SourceManifest) -> Result<String> {
    // The prepared manifest pins the source shape and column contract. Row counts and
    // asset contents are allowed to change while the old service remains online; those
    // values are verified against the actual seed/catch-up snapshot instead.
    let mut structural = manifest.clone();
    for table in &mut structural.tables {
        table.row_count = 0;
    }
    structural.assets.clear();
    let canonical = serde_json::to_vec(&structural)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn classify_source_object(name: &str) -> &'static str {
    let name = name.to_ascii_lowercase();
    if name.contains("archive")
        || name.contains("rollup")
        || name.contains("summary")
        || name.contains("timeseries")
        || name.contains("stats")
    {
        "derived"
    } else if name.contains("raw")
        || name.contains("blob")
        || name.contains("asset")
        || name.contains("file")
    {
        "immutable"
    } else if name.contains("invocation")
        || name.contains("attempt")
        || name.contains("event")
        || name.contains("timeline")
    {
        "append_only"
    } else {
        "mutable"
    }
}

async fn create_online_backup(root: &Path) -> Result<PathBuf> {
    let source = source_database_path(root);
    let destination = env::temp_dir().join(format!("cvm-migration-{}.sqlite", nanoid::nanoid!()));
    let source = source.clone();
    let destination_for_task = destination.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        use std::ffi::CString;
        use std::ptr;

        let source_c = CString::new(source.to_string_lossy().as_bytes())?;
        let destination_c = CString::new(destination_for_task.to_string_lossy().as_bytes())?;
        let mut source_db = ptr::null_mut();
        let mut destination_db = ptr::null_mut();
        let source_flags = libsqlite3_sys::SQLITE_OPEN_READONLY | libsqlite3_sys::SQLITE_OPEN_URI;
        let destination_flags = libsqlite3_sys::SQLITE_OPEN_READWRITE
            | libsqlite3_sys::SQLITE_OPEN_CREATE
            | libsqlite3_sys::SQLITE_OPEN_URI;
        let source_rc = unsafe {
            libsqlite3_sys::sqlite3_open_v2(
                source_c.as_ptr(),
                &mut source_db,
                source_flags,
                ptr::null(),
            )
        };
        if source_rc != libsqlite3_sys::SQLITE_OK {
            bail!("SQLite online backup could not open source: code {source_rc}");
        }
        let destination_rc = unsafe {
            libsqlite3_sys::sqlite3_open_v2(
                destination_c.as_ptr(),
                &mut destination_db,
                destination_flags,
                ptr::null(),
            )
        };
        if destination_rc != libsqlite3_sys::SQLITE_OK {
            unsafe { libsqlite3_sys::sqlite3_close(source_db) };
            bail!("SQLite online backup could not open destination: code {destination_rc}");
        }
        let main = CString::new("main")?;
        let backup = unsafe {
            libsqlite3_sys::sqlite3_backup_init(
                destination_db,
                main.as_ptr(),
                source_db,
                main.as_ptr(),
            )
        };
        if backup.is_null() {
            unsafe {
                libsqlite3_sys::sqlite3_close(destination_db);
                libsqlite3_sys::sqlite3_close(source_db);
            }
            bail!("SQLite online backup initialization failed");
        }
        let mut step_rc;
        loop {
            step_rc = unsafe { libsqlite3_sys::sqlite3_backup_step(backup, 128) };
            if step_rc == libsqlite3_sys::SQLITE_DONE {
                break;
            }
            if step_rc == libsqlite3_sys::SQLITE_OK
                || step_rc == libsqlite3_sys::SQLITE_BUSY
                || step_rc == libsqlite3_sys::SQLITE_LOCKED
            {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            break;
        }
        let finish_rc = unsafe { libsqlite3_sys::sqlite3_backup_finish(backup) };
        unsafe {
            libsqlite3_sys::sqlite3_close(destination_db);
            libsqlite3_sys::sqlite3_close(source_db);
        }
        if step_rc != libsqlite3_sys::SQLITE_DONE || finish_rc != libsqlite3_sys::SQLITE_OK {
            bail!("SQLite online backup failed: step={step_rc} finish={finish_rc}");
        }
        Ok(())
    })
    .await
    .context("SQLite online backup task failed")??;
    Ok(destination)
}

struct SnapshotGuard(PathBuf);

impl SnapshotGuard {
    fn new(path: PathBuf, cleanup: bool) -> Self {
        if cleanup {
            Self(path)
        } else {
            Self(PathBuf::new())
        }
    }
}

impl Drop for SnapshotGuard {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
        }
    }
}

async fn migration_snapshot(root: &Path) -> Result<(PathBuf, bool)> {
    if let Ok(path) = env::var("MIGRATION_SNAPSHOT_PATH") {
        let path = PathBuf::from(path);
        if !path.is_file() {
            bail!(
                "MIGRATION_SNAPSHOT_PATH does not point to a file: {}",
                path.display()
            );
        }
        return Ok((path, false));
    }
    Ok((create_online_backup(root).await?, true))
}

fn collect_asset_files(
    root: &Path,
    base: &Path,
    kind: &str,
    assets: &mut Vec<SourceAsset>,
) -> Result<()> {
    let mut entries = fs::read_dir(root)
        .with_context(|| format!("failed to read asset directory {}", root.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_asset_files(&path, base, kind, assets)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read asset {}", path.display()))?;
        let relative_path = path
            .strip_prefix(base)
            .with_context(|| format!("asset path escaped source root: {}", path.display()))?
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        assets.push(SourceAsset {
            relative_path,
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            asset_kind: kind.to_owned(),
        });
    }
    Ok(())
}

fn collect_source_assets(source_root: &Path) -> Result<Vec<SourceAsset>> {
    let base = if source_root.is_dir() {
        source_root.to_path_buf()
    } else {
        source_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let candidates = [("proxy_raw_payloads", "proxy_raw"), ("archives", "archive")];
    let mut assets = Vec::new();
    for (directory, kind) in candidates {
        let path = base.join(directory);
        if path.is_dir() {
            collect_asset_files(&path, &base, kind, &mut assets)?;
        }
    }
    assets.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(assets)
}

async fn inspect_source_database(
    pool: &sqlx::SqlitePool,
    source_root: &Path,
) -> Result<SourceManifest> {
    let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .fetch_all(pool)
        .await?;
    let mut tables = Vec::with_capacity(rows.len());
    let mut foreign_keys = Vec::new();
    let mut unique_indexes = Vec::new();
    let partial_predicate_pattern = regex::Regex::new(r"(?i)\bWHERE\b")?;
    for row in rows {
        let name: String = row.try_get("name")?;
        let escaped = name.replace('"', "\"\"");
        let mut columns = Vec::new();
        let mut primary_key = Vec::new();
        for column in sqlx::query(&format!("PRAGMA table_info(\"{escaped}\")"))
            .fetch_all(pool)
            .await?
        {
            let column_name: String = column.try_get("name")?;
            let declared_type: String = column
                .try_get::<Option<String>, _>("type")?
                .unwrap_or_default();
            let primary_key_position: i64 = column.try_get("pk")?;
            columns.push(SourceColumn {
                name: column_name.clone(),
                declared_type,
            });
            if primary_key_position > 0 {
                primary_key.push((primary_key_position, column_name));
            }
        }
        primary_key.sort_by_key(|(position, _)| *position);
        let count = if env::var_os("MIGRATION_SKIP_SOURCE_COUNTS").is_some() {
            0
        } else {
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{escaped}\""))
                .fetch_one(pool)
                .await?
        };
        for foreign_key in sqlx::query(&format!("PRAGMA foreign_key_list(\"{escaped}\")"))
            .fetch_all(pool)
            .await?
        {
            let constraint_id: i64 = foreign_key.try_get("id")?;
            let sequence: i64 = foreign_key.try_get("seq")?;
            let column: String = foreign_key.try_get("from")?;
            let referenced_table: String = foreign_key.try_get("table")?;
            let referenced_column: String = foreign_key.try_get("to")?;
            let on_update: String = foreign_key.try_get("on_update")?;
            let on_delete: String = foreign_key.try_get("on_delete")?;
            foreign_keys.push(SourceForeignKey {
                table: name.clone(),
                constraint_id,
                sequence,
                column,
                referenced_table,
                referenced_column,
                on_update,
                on_delete,
            });
        }
        for index in sqlx::query(&format!("PRAGMA index_list(\"{escaped}\")"))
            .fetch_all(pool)
            .await?
        {
            let index_name: String = index.try_get("name")?;
            let is_unique: i64 = index.try_get("unique")?;
            if is_unique == 0 || index_name.starts_with("sqlite_autoindex_") {
                continue;
            }
            let is_partial: i64 = index.try_get("partial").unwrap_or(0);
            let predicate = if is_partial != 0 {
                let sql: String = sqlx::query_scalar(
                    "SELECT sql FROM sqlite_master WHERE type='index' AND name=?1",
                )
                .bind(&index_name)
                .fetch_one(pool)
                .await?;
                let position = partial_predicate_pattern
                    .find(&sql)
                    .context("partial unique index has no WHERE predicate")?;
                Some(sql[position.end()..].trim().to_owned())
            } else {
                None
            };
            let mut index_columns: Vec<(i64, String)> = Vec::new();
            for index_column in sqlx::query(&format!(
                "PRAGMA index_info(\"{}\")",
                index_name.replace('\"', "\"\"")
            ))
            .fetch_all(pool)
            .await?
            {
                let sequence: i64 = index_column.try_get("seqno")?;
                let column: Option<String> = index_column.try_get("name")?;
                let column = column.with_context(|| {
                    format!(
                        "source unique index {name}.{index_name} uses an expression and cannot be materialized safely"
                    )
                })?;
                index_columns.push((sequence, column));
            }
            index_columns.sort_by_key(|(sequence, _)| *sequence);
            unique_indexes.push(SourceUniqueIndex {
                table: name.clone(),
                name: index_name,
                columns: index_columns
                    .into_iter()
                    .map(|(_, column)| column)
                    .collect(),
                predicate,
            });
        }
        let kind = classify_source_object(&name);
        tables.push(SourceTable {
            name,
            columns,
            primary_key: primary_key.into_iter().map(|(_, column)| column).collect(),
            row_count: count,
            kind: kind.to_owned(),
        });
    }
    foreign_keys.sort_by(|left, right| {
        (
            &left.table,
            left.constraint_id,
            left.sequence,
            &left.column,
            &left.referenced_table,
            &left.referenced_column,
        )
            .cmp(&(
                &right.table,
                right.constraint_id,
                right.sequence,
                &right.column,
                &right.referenced_table,
                &right.referenced_column,
            ))
    });
    unique_indexes
        .sort_by(|left, right| (&left.table, &left.name).cmp(&(&right.table, &right.name)));
    let manifest = SourceManifest {
        format_version: 2,
        source_root: source_root.display().to_string(),
        tables,
        foreign_keys,
        unique_indexes,
        assets: collect_source_assets(source_root)?,
    };
    Ok(manifest)
}

async fn inspect_source(root: &Path) -> Result<SourceManifest> {
    let (snapshot, cleanup) = migration_snapshot(root).await?;
    let _snapshot_guard = SnapshotGuard::new(snapshot.clone(), cleanup);
    inspect_source_snapshot(&snapshot, root).await
}

async fn inspect_source_snapshot(snapshot: &Path, source_root: &Path) -> Result<SourceManifest> {
    let pool = source_pool(snapshot).await?;
    let result = inspect_source_database(&pool, source_root).await;
    pool.close().await;
    result
}

async fn assert_manifest_matches_prepared_run(
    pool: &sqlx::PgPool,
    run_id: &str,
    manifest: &SourceManifest,
) -> Result<()> {
    let expected: String =
        sqlx::query_scalar("SELECT source_manifest_sha256 FROM cvm_migration.runs WHERE run_id=$1")
            .bind(run_id)
            .fetch_one(pool)
            .await
            .context("failed to load prepared source manifest checksum")?;
    let actual = manifest_checksum(manifest)?;
    if actual != expected {
        bail!(
            "source manifest checksum does not match prepared run {run_id}; inspect the source again and prepare a new run"
        );
    }
    Ok(())
}

async fn encode_row(
    row: &sqlx::sqlite::SqliteRow,
    table: &SourceTable,
) -> Result<serde_json::Value> {
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        if !table
            .columns
            .iter()
            .any(|source_column| source_column.name == column.name())
        {
            continue;
        }
        let declared_type = table
            .columns
            .iter()
            .find(|source_column| source_column.name == column.name())
            .map(|source_column| source_column.declared_type.to_ascii_uppercase())
            .unwrap_or_default();
        let value = if declared_type.contains("BLOB") {
            row.try_get::<Option<Vec<u8>>, _>(index).map(|value| {
                value.map_or(serde_json::Value::Null, |bytes| {
                    serde_json::Value::String(
                        base64::engine::general_purpose::STANDARD.encode(bytes),
                    )
                })
            })?
        } else if declared_type.contains("REAL")
            || declared_type.contains("FLOA")
            || declared_type.contains("DOUB")
        {
            row.try_get::<Option<f64>, _>(index)
                .map(|value| value.map_or(serde_json::Value::Null, serde_json::Value::from))?
        } else if declared_type.contains("NUM") || declared_type.contains("DEC") {
            if let Ok(value) = row.try_get::<Option<String>, _>(index) {
                value.map_or(serde_json::Value::Null, serde_json::Value::String)
            } else if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
                value.map_or(serde_json::Value::Null, serde_json::Value::from)
            } else {
                serde_json::Value::Null
            }
        } else if declared_type.contains("INT") || declared_type.contains("BOOL") {
            if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
                value.map_or(serde_json::Value::Null, serde_json::Value::from)
            } else if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
                value.map_or(serde_json::Value::Null, serde_json::Value::from)
            } else {
                row.try_get::<Option<String>, _>(index)
                    .map(|value| value.map_or(serde_json::Value::Null, serde_json::Value::String))?
            }
        } else if let Ok(value) = row.try_get::<Option<String>, _>(index) {
            value.map_or(serde_json::Value::Null, serde_json::Value::String)
        } else if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
            value.map_or(serde_json::Value::Null, |bytes| {
                serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(bytes))
            })
        } else {
            serde_json::Value::Null
        };
        object.insert(column.name().to_string(), value);
    }
    let mut payload = serde_json::Value::Object(object);
    sanitize_postgres_json(&mut payload);
    Ok(payload)
}

fn source_key(payload: &serde_json::Value, table: &SourceTable, row_number: i64) -> Result<String> {
    if table.primary_key.is_empty() {
        return Ok(format!("row:{row_number}"));
    }
    let object = payload
        .as_object()
        .context("encoded source row is not a JSON object")?;
    let mut key = serde_json::Map::new();
    for column in &table.primary_key {
        let value = object
            .get(column)
            .with_context(|| format!("primary key column {column} is absent from source row"))?;
        key.insert(column.clone(), value.clone());
    }
    Ok(serde_json::to_string(&key)?)
}

fn sanitize_postgres_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            if text.contains('\0') {
                *text = text.replace('\0', "\u{FFFD}");
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                sanitize_postgres_json(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                sanitize_postgres_json(value);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn target_schema_for_object(object_name: &str) -> &'static str {
    let name = object_name.to_ascii_lowercase();
    if name.contains("archive") {
        "cvm_archive"
    } else if name.contains("asset") || name.contains("file") || name.contains("raw") {
        "cvm_assets"
    } else if name.contains("stat") || name.contains("rollup") || name.contains("metric") {
        "cvm_stats"
    } else if name.contains("invocation")
        || name.contains("attempt")
        || name.contains("proxy")
        || name.contains("activity")
    {
        "cvm_activity"
    } else {
        "cvm_core"
    }
}

fn target_table_for_object(object_name: &str) -> String {
    if object_name.chars().count() <= 55 {
        object_name.to_owned()
    } else {
        let digest = Sha256::digest(object_name.as_bytes());
        let digest = format!("{digest:x}");
        format!("source_{}", &digest[..24])
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('\"', "\"\""))
}

fn quote_literal(literal: &str) -> String {
    format!("'{}'", literal.replace('\'', "''"))
}

pub(crate) fn quote_literal_for_store(literal: &str) -> String {
    quote_literal(literal)
}

fn source_order_by(table: &SourceTable) -> String {
    if table.primary_key.is_empty() {
        "rowid".to_owned()
    } else {
        table
            .primary_key
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn postgres_type_for_source_column(declared_type: &str) -> &'static str {
    let normalized = declared_type.to_ascii_uppercase();
    if normalized.contains("INT") {
        "BIGINT"
    } else if normalized.contains("REAL")
        || normalized.contains("FLOA")
        || normalized.contains("DOUB")
    {
        "DOUBLE PRECISION"
    } else if normalized.contains("NUM") || normalized.contains("DEC") {
        "NUMERIC"
    } else if normalized.contains("BLOB") {
        "BYTEA"
    } else {
        "TEXT"
    }
}

pub(crate) fn postgres_type_for_source_column_for_store(declared_type: &str) -> &'static str {
    postgres_type_for_source_column(declared_type)
}

fn copy_csv_field(value: &str, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        if character == '"' {
            output.push('"');
        }
        output.push(character);
    }
    output.push('"');
}

async fn stage_source_rows_from_snapshot(
    pool: &sqlx::PgPool,
    run_id: &str,
    tables: &[SourceTable],
    batch_rows: u64,
    snapshot: &Path,
) -> Result<()> {
    direct::import_snapshot(pool, run_id, tables, batch_rows, snapshot).await
}

async fn transition(
    pool: &sqlx::PgPool,
    run_id: &str,
    expected: &[&str],
    next: &str,
) -> Result<()> {
    let state: Option<String> =
        sqlx::query_scalar("SELECT state FROM cvm_migration.runs WHERE run_id=$1")
            .bind(run_id)
            .fetch_optional(pool)
            .await?;
    let state = state.context("migration run does not exist")?;
    if !is_valid_transition(&state, expected) {
        bail!("invalid migration state transition: {state} -> {next}");
    }
    sqlx::query("UPDATE cvm_migration.runs SET state=$2, resume_state=NULL, updated_at=now() WHERE run_id=$1")
        .bind(run_id)
        .bind(next)
        .execute(pool)
        .await?;
    Ok(())
}

fn is_valid_transition(state: &str, expected: &[&str]) -> bool {
    expected.contains(&state)
}

fn resumable_state(state: &str, resume_state: Option<&str>) -> Result<String> {
    if state == "seeding" {
        return Ok("prepared".to_owned());
    }
    if state == "catching_up" {
        return Ok("source_stopped".to_owned());
    }
    if !matches!(state, "paused" | "failed") {
        bail!("migration run is not resumable: {state}");
    }
    let next_state = resume_state.unwrap_or("prepared");
    if !matches!(next_state, "prepared" | "source_stopped") {
        bail!("migration resume state is invalid: {next_state}");
    }
    Ok(next_state.to_owned())
}

async fn ensure_run_profile(pool: &sqlx::PgPool, run_id: &str, target_profile: &str) -> Result<()> {
    let stored: String =
        sqlx::query_scalar("SELECT target_profile FROM cvm_migration.runs WHERE run_id=$1")
            .bind(run_id)
            .fetch_one(pool)
            .await
            .with_context(|| format!("migration run {run_id} does not exist"))?;
    if stored != target_profile {
        bail!("migration run {run_id} belongs to target profile {stored}, not {target_profile}");
    }
    Ok(())
}

async fn merge_staged_rows(pool: &sqlx::PgPool, run_id: &str) -> Result<()> {
    finish_domain_tables(pool, run_id).await
}

async fn clear_missing_source_objects(
    pool: &sqlx::PgPool,
    run_id: &str,
    source_objects: &HashSet<String>,
) -> Result<()> {
    let known_objects: Vec<String> =
        sqlx::query_scalar("SELECT object_name FROM cvm_migration.objects WHERE run_id=$1")
            .bind(run_id)
            .fetch_all(pool)
            .await?;
    for object_name in known_objects {
        if source_objects.contains(&object_name) {
            continue;
        }
        if let Some((target_schema, target_table)) = sqlx::query_as::<_, (String, String)>(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(run_id)
        .bind(&object_name)
        .fetch_optional(pool)
        .await?
        {
            let qualified = format!(
                "{}.{}",
                quote_identifier(&target_schema),
                quote_identifier(&target_table)
            );
            sqlx::query(&format!("TRUNCATE TABLE {qualified}"))
                .execute(pool)
                .await
                .with_context(|| {
                    format!(
                        "failed to remove materialized rows for missing source object {object_name}"
                    )
                })?;
        }
        for table in [
            "cvm_migration.object_columns",
            "cvm_migration.object_foreign_keys",
            "cvm_migration.object_unique_indexes",
            "cvm_migration.object_targets",
        ] {
            sqlx::query(&format!(
                "DELETE FROM {table} WHERE run_id=$1 AND object_name=$2"
            ))
            .bind(run_id)
            .bind(&object_name)
            .execute(pool)
            .await?;
        }
        sqlx::query("DELETE FROM cvm_migration.objects WHERE run_id=$1 AND object_name=$2")
            .bind(run_id)
            .bind(&object_name)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn drop_legacy_generated_indexes(
    pool: &sqlx::PgPool,
    target_schema: &str,
    target_table: &str,
) -> Result<()> {
    let legacy_indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexname FROM pg_indexes WHERE schemaname=$1 AND tablename=$2 AND (indexname LIKE 'cvm_pk_%' OR indexname LIKE 'cvm_uq_%') AND indexname NOT LIKE '%_v2_%'",
    )
    .bind(target_schema)
    .bind(target_table)
    .fetch_all(pool)
    .await?;
    for index_name in legacy_indexes {
        sqlx::raw_sql(&format!(
            "DROP INDEX IF EXISTS {}.{}",
            quote_identifier(target_schema),
            quote_identifier(&index_name)
        ))
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn ensure_domain_tables(pool: &sqlx::PgPool, run_id: &str) -> Result<()> {
    let objects: Vec<String> = sqlx::query_scalar(
        "SELECT object_name FROM cvm_migration.objects WHERE run_id=$1 ORDER BY object_name",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    for object_name in objects {
        let target_schema = target_schema_for_object(&object_name);
        let target_table = target_table_for_object(&object_name);
        let qualified = format!(
            "{}.{}",
            quote_identifier(target_schema),
            quote_identifier(&target_table)
        );
        let columns: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT column_name, declared_type, primary_key_position FROM cvm_migration.object_columns WHERE run_id=$1 AND object_name=$2 ORDER BY ordinal",
        )
        .bind(run_id)
        .bind(&object_name)
        .fetch_all(pool)
        .await?;
        let mut definitions = Vec::new();
        for (column_name, declared_type, primary_key_position) in &columns {
            if matches!(
                column_name.as_str(),
                "_cvm_run_id" | "_cvm_source_key" | "_cvm_payload"
            ) {
                bail!(
                    "source object {object_name} uses reserved migration column name {column_name}"
                );
            }
            let pg_type = postgres_type_for_source_column(declared_type);
            let identity = if *primary_key_position > 0 && pg_type == "BIGINT" {
                " GENERATED BY DEFAULT AS IDENTITY"
            } else {
                ""
            };
            definitions.push(format!(
                "{} {}{}",
                quote_identifier(column_name),
                pg_type,
                identity
            ));
        }
        sqlx::raw_sql(&format!(
            "CREATE TABLE IF NOT EXISTS {qualified} ({})",
            definitions.join(", ")
        ))
        .execute(pool)
        .await
        .with_context(|| format!("failed to create target table {target_schema}.{target_table}"))?;
        for (column_name, declared_type, _) in &columns {
            sqlx::query(&format!(
                "ALTER TABLE {qualified} ADD COLUMN IF NOT EXISTS {} {}",
                quote_identifier(column_name),
                postgres_type_for_source_column(declared_type)
            ))
            .execute(pool)
            .await?;
        }
        drop_legacy_generated_indexes(pool, target_schema, &target_table).await?;
        sqlx::query("INSERT INTO cvm_migration.object_targets(run_id,object_name,target_schema,target_table) VALUES($1,$2,$3,$4) ON CONFLICT(run_id,object_name) DO UPDATE SET target_schema=EXCLUDED.target_schema,target_table=EXCLUDED.target_table")
            .bind(run_id)
            .bind(&object_name)
            .bind(target_schema)
            .bind(&target_table)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn finish_domain_tables(pool: &sqlx::PgPool, run_id: &str) -> Result<()> {
    let targets: Vec<(String,String,String)> = sqlx::query_as("SELECT object_name,target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1").bind(run_id).fetch_all(pool).await?;
    for (object_name, target_schema, target_table) in targets {
        let qualified = format!(
            "{}.{}",
            quote_identifier(&target_schema),
            quote_identifier(&target_table)
        );
        let columns: Vec<(String,String,i64)> = sqlx::query_as("SELECT column_name,declared_type,primary_key_position FROM cvm_migration.object_columns WHERE run_id=$1 AND object_name=$2 ORDER BY ordinal").bind(run_id).bind(&object_name).fetch_all(pool).await?;
        let mut key_columns = columns
            .iter()
            .filter(|(_, _, position)| *position > 0)
            .collect::<Vec<_>>();
        key_columns.sort_by_key(|(_, _, position)| *position);
        let primary_key_columns = key_columns
            .iter()
            .map(|(name, _, _)| quote_identifier(name))
            .collect::<Vec<_>>();
        if !primary_key_columns.is_empty() {
            let index_digest = format!("{:x}", Sha256::digest(object_name.as_bytes()));
            let legacy_index_name = format!("cvm_pk_{}", &index_digest[..24]);
            let index_name = format!("cvm_pk_v2_{}", &index_digest[..22]);
            sqlx::raw_sql(&format!(
                "DROP INDEX IF EXISTS {}.{}",
                quote_identifier(&target_schema),
                quote_identifier(&legacy_index_name)
            ))
            .execute(pool)
            .await?;
            sqlx::raw_sql(&format!(
                "CREATE UNIQUE INDEX IF NOT EXISTS {} ON {qualified} ({})",
                quote_identifier(&index_name),
                primary_key_columns.join(", ")
            ))
            .execute(pool)
            .await?;
        }
        for (column_name, declared_type, primary_key_position) in &columns {
            if *primary_key_position == 0
                || postgres_type_for_source_column(declared_type) != "BIGINT"
            {
                continue;
            }
            let sequence_name: Option<String> =
                sqlx::query_scalar("SELECT pg_get_serial_sequence($1, $2)")
                    .bind(format!("{target_schema}.{target_table}"))
                    .bind(column_name)
                    .fetch_one(pool)
                    .await?;
            if let Some(sequence_name) = sequence_name {
                let maximum: Option<i64> = sqlx::query_scalar(&format!(
                    "SELECT MAX({}) FROM {qualified}",
                    quote_identifier(column_name)
                ))
                .fetch_one(pool)
                .await?;
                let value = maximum.unwrap_or(1).max(1);
                sqlx::query("SELECT setval($1, $2, $3)")
                    .bind(sequence_name)
                    .bind(value)
                    .bind(maximum.is_some())
                    .execute(pool)
                    .await?;
            }
        }
    }
    materialize_unique_indexes(pool, run_id).await?;
    materialize_foreign_keys(pool, run_id).await?;
    Ok(())
}

async fn materialize_unique_indexes(pool: &sqlx::PgPool, run_id: &str) -> Result<()> {
    let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
        "SELECT object_name,index_name,sequence_no,column_name FROM cvm_migration.object_unique_indexes WHERE run_id=$1 ORDER BY object_name,index_name,sequence_no",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    let mut grouped: BTreeMap<(String, String), Vec<(i64, String)>> = BTreeMap::new();
    for (object_name, index_name, sequence_no, column_name) in rows {
        grouped
            .entry((object_name, index_name))
            .or_default()
            .push((sequence_no, column_name));
    }
    for ((object_name, index_name), mut columns) in grouped {
        columns.sort_by_key(|(sequence_no, _)| *sequence_no);
        let target: (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(run_id)
        .bind(&object_name)
        .fetch_one(pool)
        .await
        .with_context(|| format!("missing target for unique index {object_name}.{index_name}"))?;
        let qualified = format!(
            "{}.{}",
            quote_identifier(&target.0),
            quote_identifier(&target.1)
        );
        let digest = Sha256::digest(format!("{run_id}:{object_name}:{index_name}").as_bytes());
        let legacy_index_name = format!("cvm_uq_{:x}", digest)[..31].to_owned();
        let target_index_name = format!("cvm_uq_v2_{:x}", digest)[..31].to_owned();
        let columns = columns
            .iter()
            .map(|(_, column)| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        sqlx::raw_sql(&format!(
            "DROP INDEX IF EXISTS {}.{}",
            quote_identifier(&target.0),
            quote_identifier(&legacy_index_name)
        ))
        .execute(pool)
        .await?;
        let predicate: Option<String> = sqlx::query_scalar("SELECT predicate FROM cvm_migration.object_unique_indexes WHERE run_id=$1 AND object_name=$2 AND index_name=$3 LIMIT 1").bind(run_id).bind(&object_name).bind(&index_name).fetch_one(pool).await?;
        let predicate = predicate.map(|p| format!(" WHERE {p}")).unwrap_or_default();
        sqlx::raw_sql(&format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS {} ON {qualified} ({columns}){predicate}",
            quote_identifier(&target_index_name)
        ))
        .execute(pool)
        .await
        .with_context(|| {
            format!("failed to materialize unique index {object_name}.{index_name}")
        })?;
    }
    Ok(())
}

async fn materialize_foreign_keys(pool: &sqlx::PgPool, run_id: &str) -> Result<()> {
    type ForeignKeyRow = (
        String,
        Option<i64>,
        Option<i64>,
        String,
        String,
        String,
        String,
        String,
    );
    type ForeignKeyGroup = (String, i64, String, String, String);
    type ForeignKeyColumns = Vec<(i64, String, String)>;
    let rows: Vec<ForeignKeyRow> = sqlx::query_as(
        "SELECT object_name,constraint_id,sequence_no,column_name,referenced_object_name,referenced_column_name,on_update,on_delete FROM cvm_migration.object_foreign_keys WHERE run_id=$1 ORDER BY object_name,constraint_id,sequence_no",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    let mut grouped: BTreeMap<ForeignKeyGroup, ForeignKeyColumns> = BTreeMap::new();
    for (
        object_name,
        constraint_id,
        sequence_no,
        column_name,
        referenced_object_name,
        referenced_column_name,
        on_update,
        on_delete,
    ) in rows
    {
        let constraint_id =
            constraint_id.context("migration foreign key is missing constraint id")?;
        let sequence_no =
            sequence_no.context("migration foreign key is missing sequence number")?;
        grouped
            .entry((
                object_name,
                constraint_id,
                referenced_object_name,
                on_update,
                on_delete,
            ))
            .or_default()
            .push((sequence_no, column_name, referenced_column_name));
    }
    for ((object_name, constraint_id, referenced_object_name, on_update, on_delete), mut columns) in
        grouped
    {
        columns.sort_by_key(|(sequence, _, _)| *sequence);
        let source_target: Option<(String, String)> = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(run_id)
        .bind(&object_name)
        .fetch_optional(pool)
        .await?;
        let (source_schema, source_table) = source_target.with_context(|| {
            format!("foreign key source object is not materialized: {object_name}")
        })?;
        let referenced_target: Option<(String, String)> = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(run_id)
        .bind(&referenced_object_name)
        .fetch_optional(pool)
        .await?;
        let (referenced_schema, referenced_table) = referenced_target.with_context(|| {
            format!("foreign key referenced object is not materialized: {referenced_object_name}")
        })?;
        let normalize_action = |action: &str| -> Result<&'static str> {
            match action.to_ascii_uppercase().as_str() {
                "NO ACTION" => Ok("NO ACTION"),
                "RESTRICT" => Ok("RESTRICT"),
                "CASCADE" => Ok("CASCADE"),
                "SET NULL" => Ok("SET NULL"),
                "SET DEFAULT" => Ok("SET DEFAULT"),
                other => bail!("unsupported SQLite foreign-key action: {other}"),
            }
        };
        let on_update = normalize_action(&on_update)?;
        let on_delete = normalize_action(&on_delete)?;
        let source_columns = columns
            .iter()
            .map(|(_, column, _)| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let referenced_columns = columns
            .iter()
            .map(|(_, _, column)| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let constraint_name = foreign_key_constraint_name(
            run_id,
            &object_name,
            constraint_id,
            &referenced_object_name,
        );
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_constraint c JOIN pg_class r ON r.oid=c.conrelid JOIN pg_namespace n ON n.oid=r.relnamespace WHERE n.nspname=$1 AND r.relname=$2 AND c.conname=$3)",
        )
        .bind(&source_schema)
        .bind(&source_table)
        .bind(&constraint_name)
        .fetch_one(pool)
        .await?;
        if exists {
            continue;
        }
        let source_qualified = format!(
            "{}.{}",
            quote_identifier(&source_schema),
            quote_identifier(&source_table)
        );
        let referenced_qualified = format!(
            "{}.{}",
            quote_identifier(&referenced_schema),
            quote_identifier(&referenced_table)
        );
        sqlx::raw_sql(&format!(
            "ALTER TABLE {source_qualified} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {referenced_qualified} ({}) ON UPDATE {on_update} ON DELETE {on_delete}",
            quote_identifier(&constraint_name),
            source_columns,
            referenced_columns,
        ))
        .execute(pool)
        .await
        .with_context(|| format!("failed to materialize foreign key on {object_name}"))?;
    }
    Ok(())
}

fn foreign_key_constraint_name(
    run_id: &str,
    object_name: &str,
    constraint_id: i64,
    referenced_object_name: &str,
) -> String {
    let digest = Sha256::digest(
        format!("{run_id}:{object_name}:{constraint_id}:{referenced_object_name}").as_bytes(),
    );
    format!("cvm_fk_{:x}", digest)[..31].to_owned()
}

async fn verify_imported_rows(
    pool: &sqlx::PgPool,
    run_id: &str,
    strict: bool,
    source_root: Option<&Path>,
    snapshot: Option<&Path>,
) -> Result<()> {
    let mismatches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cvm_migration.objects WHERE run_id=$1 AND source_rows <> imported_rows",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    if mismatches != 0 {
        bail!("migration row count verification failed: {mismatches} object(s) differ");
    }
    if strict {
        let source_root =
            source_root.context("strict migration verification requires a recorded source root")?;
        verify_asset_records(pool, run_id, source_root).await?;
        verify_direct_payloads(pool, run_id, source_root, snapshot).await?;
    }
    let targets: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT object_name,target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    for (object_name, target_schema, target_table) in targets {
        let qualified = format!(
            "{}.{}",
            quote_identifier(&target_schema),
            quote_identifier(&target_table)
        );
        let imported_count: i64 = sqlx::query_scalar(
            "SELECT imported_rows FROM cvm_migration.objects WHERE run_id=$1 AND object_name=$2",
        )
        .bind(run_id)
        .bind(&object_name)
        .fetch_one(pool)
        .await?;
        let target_count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {qualified}"))
            .fetch_one(pool)
            .await?;
        if imported_count != target_count {
            bail!("target materialization verification failed for {object_name}");
        }
        verify_target_sequences(pool, run_id, &object_name, &target_schema, &target_table).await?;
    }
    direct::enforce_capacity(pool).await?;
    verify_target_foreign_keys(pool, run_id).await?;
    verify_target_unique_indexes(pool, run_id).await?;
    Ok(())
}

async fn verify_target_foreign_keys(pool: &sqlx::PgPool, run_id: &str) -> Result<()> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT DISTINCT object_name,constraint_id,referenced_object_name FROM cvm_migration.object_foreign_keys WHERE run_id=$1 ORDER BY object_name,constraint_id,referenced_object_name",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    for (object_name, constraint_id, referenced_object_name) in rows {
        let source_target: (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(run_id)
        .bind(&object_name)
        .fetch_one(pool)
        .await
        .with_context(|| format!("missing target for foreign key source object {object_name}"))?;
        let referenced_target: (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(run_id)
        .bind(&referenced_object_name)
        .fetch_one(pool)
        .await
        .with_context(|| {
            format!("missing target for foreign key reference {referenced_object_name}")
        })?;
        let constraint_name = foreign_key_constraint_name(
            run_id,
            &object_name,
            constraint_id,
            &referenced_object_name,
        );
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_constraint c JOIN pg_class r ON r.oid=c.conrelid JOIN pg_namespace n ON n.oid=r.relnamespace JOIN pg_class ref_r ON ref_r.oid=c.confrelid JOIN pg_namespace ref_n ON ref_n.oid=ref_r.relnamespace WHERE n.nspname=$1 AND r.relname=$2 AND c.conname=$3 AND ref_n.nspname=$4 AND ref_r.relname=$5)",
        )
        .bind(&source_target.0)
        .bind(&source_target.1)
        .bind(&constraint_name)
        .bind(&referenced_target.0)
        .bind(&referenced_target.1)
        .fetch_one(pool)
        .await?;
        if !exists {
            bail!(
                "target foreign key verification failed for {object_name} -> {referenced_object_name}"
            );
        }
    }
    Ok(())
}

async fn verify_target_unique_indexes(pool: &sqlx::PgPool, run_id: &str) -> Result<()> {
    let indexes: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT object_name,index_name FROM cvm_migration.object_unique_indexes WHERE run_id=$1 ORDER BY object_name,index_name",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    for (object_name, index_name) in indexes {
        let target: (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(run_id)
        .bind(&object_name)
        .fetch_one(pool)
        .await?;
        let digest = Sha256::digest(format!("{run_id}:{object_name}:{index_name}").as_bytes());
        let target_index_name = format!("cvm_uq_v2_{:x}", digest)[..31].to_owned();
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_class i JOIN pg_index ix ON ix.indexrelid=i.oid JOIN pg_class t ON t.oid=ix.indrelid JOIN pg_namespace n ON n.oid=t.relnamespace WHERE n.nspname=$1 AND t.relname=$2 AND i.relname=$3 AND ix.indisunique)",
        )
        .bind(&target.0)
        .bind(&target.1)
        .bind(&target_index_name)
        .fetch_one(pool)
        .await?;
        if !exists {
            bail!("target unique index verification failed for {object_name}.{index_name}");
        }
    }
    Ok(())
}

async fn verify_asset_records(pool: &sqlx::PgPool, run_id: &str, source_root: &Path) -> Result<()> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT relative_path,size_bytes,sha256 FROM cvm_assets.asset_index WHERE run_id=$1 ORDER BY relative_path",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    for (relative_path, size_bytes, sha256) in rows {
        let path = Path::new(&relative_path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            bail!("migration asset path is not relative and safe: {relative_path}");
        }
        if size_bytes < 0
            || sha256.len() != 64
            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("migration asset metadata is invalid: {relative_path}");
        }
        let base = if source_root.is_file() {
            source_root.parent().unwrap_or(source_root)
        } else {
            source_root
        };
        let asset_path = base.join(&relative_path);
        let canonical_base = fs::canonicalize(base).with_context(|| {
            format!("failed to resolve migration asset base {}", base.display())
        })?;
        let canonical_asset = fs::canonicalize(&asset_path)
            .with_context(|| format!("migration asset is missing: {}", asset_path.display()))?;
        if !canonical_asset.starts_with(&canonical_base) {
            bail!("migration asset escapes source root: {relative_path}");
        }
        let bytes = fs::read(&canonical_asset)
            .with_context(|| format!("failed to read migration asset {}", asset_path.display()))?;
        if bytes.len() as i64 != size_bytes || format!("{:x}", Sha256::digest(&bytes)) != sha256 {
            bail!("migration asset checksum mismatch: {relative_path}");
        }
    }
    Ok(())
}

async fn verify_target_sequences(
    pool: &sqlx::PgPool,
    run_id: &str,
    object_name: &str,
    target_schema: &str,
    target_table: &str,
) -> Result<()> {
    let columns: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT column_name, declared_type, primary_key_position FROM cvm_migration.object_columns WHERE run_id=$1 AND object_name=$2 AND primary_key_position > 0",
    )
    .bind(run_id)
    .bind(object_name)
    .fetch_all(pool)
    .await?;
    let qualified = format!(
        "{}.{}",
        quote_identifier(target_schema),
        quote_identifier(target_table)
    );
    for (column_name, declared_type, _) in columns {
        if postgres_type_for_source_column(&declared_type) != "BIGINT" {
            continue;
        }
        let sequence_name: Option<String> =
            sqlx::query_scalar("SELECT pg_get_serial_sequence($1, $2)")
                .bind(format!("{target_schema}.{target_table}"))
                .bind(&column_name)
                .fetch_one(pool)
                .await?;
        let sequence_name = sequence_name.with_context(|| {
            format!(
                "target integer primary key has no identity sequence for {object_name}.{column_name}"
            )
        })?;
        let (schema_name, sequence_name) = sequence_name
            .rsplit_once('.')
            .map(|(schema, sequence)| (schema.trim_matches('\"'), sequence.trim_matches('\"')))
            .unwrap_or((target_schema, sequence_name.as_str()));
        // Unused identity sequences have a catalog row with a NULL last_value.
        let last_value: Option<i64> = sqlx::query_scalar(
            "SELECT last_value FROM pg_sequences WHERE schemaname=$1 AND sequencename=$2",
        )
        .bind(schema_name)
        .bind(sequence_name)
        .fetch_one(pool)
        .await?;
        let maximum: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT MAX({}) FROM {qualified}",
            quote_identifier(&column_name)
        ))
        .fetch_one(pool)
        .await?;
        if let (Some(maximum), Some(last_value)) = (maximum, last_value)
            && last_value < maximum
        {
            bail!(
                "target sequence verification failed for {object_name}.{column_name}: last_value={last_value}, maximum={maximum}"
            );
        }
    }
    Ok(())
}

async fn record_failure(
    pool: &sqlx::PgPool,
    run_id: &str,
    recover_state: &str,
    error: &anyhow::Error,
) {
    let code = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<sqlx::Error>())
        .map(|_| "database_error")
        .unwrap_or("migration_error");
    let _ = sqlx::query(
        "UPDATE cvm_migration.runs SET state='failed',resume_state=$2,last_error_code=$3,updated_at=now() WHERE run_id=$1",
    )
    .bind(run_id)
    .bind(recover_state)
    .bind(code)
        .execute(pool)
    .await;
}

async fn stage_and_materialize(
    pool: &sqlx::PgPool,
    run_id: &str,
    source_root: &Path,
    batch_rows: u64,
) -> Result<()> {
    let (snapshot, cleanup) = migration_snapshot(source_root).await?;
    let _guard = SnapshotGuard::new(snapshot.clone(), cleanup);
    let manifest = inspect_source_snapshot(&snapshot, source_root).await?;
    stage_source_rows_from_snapshot(pool, run_id, &manifest.tables, batch_rows, &snapshot).await?;
    merge_staged_rows(pool, run_id).await?;
    verify_imported_rows(pool, run_id, true, Some(source_root), Some(&snapshot)).await
}

async fn register_manifest_tables(
    pool: &sqlx::PgPool,
    run_id: &str,
    tables: &[SourceTable],
    foreign_keys: &[SourceForeignKey],
    unique_indexes: &[SourceUniqueIndex],
) -> Result<()> {
    // Manifest registration used to issue every metadata statement through the pool,
    // giving each column/index its own transaction and fsync. Keep the whole manifest
    // atomic and commit it once before row staging begins.
    let mut tx = pool.begin().await?;
    for table in tables {
        sqlx::query("INSERT INTO cvm_migration.objects(run_id,object_name,source_kind,source_rows,imported_rows) VALUES($1,$2,$3,$4,0) ON CONFLICT(run_id,object_name) DO UPDATE SET source_rows=EXCLUDED.source_rows,source_kind=EXCLUDED.source_kind,updated_at=now()")
            .bind(run_id)
            .bind(&table.name)
            .bind(&table.kind)
            .bind(table.row_count)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cvm_migration.object_columns WHERE run_id=$1 AND object_name=$2")
            .bind(run_id)
            .bind(&table.name)
            .execute(&mut *tx)
            .await?;
        for (ordinal, column) in table.columns.iter().enumerate() {
            let primary_key_position = table
                .primary_key
                .iter()
                .position(|candidate| candidate == &column.name)
                .map(|position| position as i64 + 1)
                .unwrap_or(0);
            sqlx::query("INSERT INTO cvm_migration.object_columns(run_id,object_name,ordinal,column_name,declared_type,primary_key_position) VALUES($1,$2,$3,$4,$5,$6)")
                .bind(run_id)
                .bind(&table.name)
                .bind(ordinal as i64)
                .bind(&column.name)
                .bind(&column.declared_type)
                .bind(primary_key_position)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query("DELETE FROM cvm_migration.object_foreign_keys WHERE run_id=$1")
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
    for foreign_key in foreign_keys {
        sqlx::query(
            "INSERT INTO cvm_migration.object_foreign_keys(run_id,object_name,constraint_id,sequence_no,column_name,referenced_object_name,referenced_column_name,on_update,on_delete) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(run_id)
        .bind(&foreign_key.table)
        .bind(foreign_key.constraint_id)
        .bind(foreign_key.sequence)
        .bind(&foreign_key.column)
        .bind(&foreign_key.referenced_table)
        .bind(&foreign_key.referenced_column)
        .bind(&foreign_key.on_update)
        .bind(&foreign_key.on_delete)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("DELETE FROM cvm_migration.object_unique_indexes WHERE run_id=$1")
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
    for unique_index in unique_indexes {
        for (sequence_no, column_name) in unique_index.columns.iter().enumerate() {
            sqlx::query(
                "INSERT INTO cvm_migration.object_unique_indexes(run_id,object_name,index_name,sequence_no,column_name,predicate) VALUES($1,$2,$3,$4,$5,$6)",
            )
            .bind(run_id)
            .bind(&unique_index.table)
            .bind(&unique_index.name)
            .bind(sequence_no as i64)
            .bind(column_name)
            .bind(&unique_index.predicate)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn register_manifest_assets(
    pool: &sqlx::PgPool,
    run_id: &str,
    assets: &[SourceAsset],
) -> Result<()> {
    sqlx::query("DELETE FROM cvm_migration.assets WHERE run_id=$1")
        .bind(run_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM cvm_assets.asset_index WHERE run_id=$1")
        .bind(run_id)
        .execute(pool)
        .await?;
    for asset in assets {
        sqlx::query(
            "INSERT INTO cvm_migration.assets(run_id,relative_path,size_bytes,sha256,asset_kind) VALUES($1,$2,$3,$4,$5)",
        )
        .bind(run_id)
        .bind(&asset.relative_path)
        .bind(asset.size_bytes as i64)
        .bind(&asset.sha256)
        .bind(&asset.asset_kind)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO cvm_assets.asset_index(run_id,relative_path,size_bytes,sha256,asset_kind) VALUES($1,$2,$3,$4,$5)",
        )
        .bind(run_id)
        .bind(&asset.relative_path)
        .bind(asset.size_bytes as i64)
        .bind(&asset.sha256)
        .bind(&asset.asset_kind)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub(crate) async fn run(config: &AppConfig, command: &MigrationCommand) -> Result<()> {
    if matches!(command, MigrationCommand::InspectSource { .. }) {
        return run_unlocked(config, command).await;
    }
    use sqlx::Connection;
    let url = config
        .postgres_primary_migration_url
        .as_ref()
        .context("POSTGRES_PRIMARY_MIGRATION_URL is required")?;
    let mut guard = sqlx::PgConnection::connect(url.as_str()).await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(180018, 1)")
        .fetch_one(&mut guard)
        .await?;
    if !locked {
        bail!("another migration command owns this target database");
    }
    let result = run_unlocked(config, command).await;
    guard.close().await?;
    result
}

async fn run_unlocked(config: &AppConfig, command: &MigrationCommand) -> Result<()> {
    match command {
        MigrationCommand::InspectSource {
            source_root,
            manifest_out,
        } => {
            let manifest = inspect_source(source_root).await?;
            let text = serde_json::to_string_pretty(&manifest)?;
            if let Some(path) = manifest_out {
                fs::write(path, format!("{text}\n"))?;
            } else {
                println!("{text}");
            }
        }
        MigrationCommand::Prepare {
            run_id,
            source_manifest,
            target_profile,
        } => {
            let pool = target_pool(config).await?;
            ensure_target_schema(config, &pool).await?;
            let manifest = fs::read(source_manifest)?;
            let manifest: SourceManifest = serde_json::from_slice(&manifest)
                .context("source manifest is not valid migration manifest JSON")?;
            if manifest.format_version != 2 {
                bail!(
                    "unsupported source manifest format version {}; expected 2",
                    manifest.format_version
                );
            }
            let checksum = manifest_checksum(&manifest)?;
            let existing_run: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT state,source_manifest_sha256 FROM cvm_migration.runs WHERE run_id=$1",
            )
            .bind(run_id)
            .fetch_optional(&pool)
            .await?;
            let existing_state = existing_run.as_ref().map(|run| run.0.as_str());
            if existing_state.is_some() {
                ensure_run_profile(&pool, run_id, target_profile).await?;
            }
            if let Some(state) = existing_state
                && matches!(
                    state,
                    "finalized"
                        | "verified"
                        | "seeded"
                        | "source_stopped"
                        | "catching_up"
                        | "seeding"
                )
            {
                bail!("migration run cannot be prepared from state {state}");
            }
            if let Some((_, Some(existing_checksum))) = existing_run.as_ref()
                && existing_checksum != &checksum
            {
                bail!("migration source manifest does not match the existing run");
            }
            let other_run: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM cvm_migration.runs WHERE run_id<>$1)",
            )
            .bind(run_id)
            .fetch_one(&pool)
            .await?;
            if other_run {
                bail!(
                    "one primary database supports one migration run; use a separate database for another source"
                );
            }
            sqlx::query("INSERT INTO cvm_migration.runs(run_id,target_profile,source_root,source_manifest_sha256,state,resume_state,last_error_code) VALUES($1,$2,$3,$4,'prepared',NULL,NULL) ON CONFLICT(run_id) DO UPDATE SET source_root=EXCLUDED.source_root,source_manifest_sha256=EXCLUDED.source_manifest_sha256,resume_state=NULL,last_error_code=NULL,updated_at=now()")
                .bind(run_id).bind(target_profile).bind(&manifest.source_root).bind(checksum).execute(&pool).await?;
            println!("{}", json!({"runId": run_id, "state": "prepared"}));
        }
        MigrationCommand::Seed {
            run_id,
            source_root,
            batch_rows,
            target_profile,
        } => {
            let pool = target_pool(config).await?;
            ensure_target_schema(config, &pool).await?;
            ensure_run_profile(&pool, run_id, target_profile).await?;
            let (snapshot, cleanup) = migration_snapshot(source_root).await?;
            let _snapshot_guard = SnapshotGuard::new(snapshot.clone(), cleanup);
            let manifest = inspect_source_snapshot(&snapshot, source_root).await?;
            assert_manifest_matches_prepared_run(&pool, run_id, &manifest).await?;
            transition(&pool, run_id, &["prepared", "paused", "failed"], "seeding").await?;
            register_manifest_tables(
                &pool,
                run_id,
                &manifest.tables,
                &manifest.foreign_keys,
                &manifest.unique_indexes,
            )
            .await?;
            register_manifest_assets(&pool, run_id, &manifest.assets).await?;
            let import_result = async {
                stage_source_rows_from_snapshot(
                    &pool,
                    run_id,
                    &manifest.tables,
                    *batch_rows,
                    &snapshot,
                )
                .await?;
                merge_staged_rows(&pool, run_id).await?;
                verify_imported_rows(&pool, run_id, true, Some(source_root), Some(&snapshot)).await
            }
            .await;
            if let Err(error) = import_result {
                record_failure(&pool, run_id, "prepared", &error).await;
                return Err(error);
            }
            transition(&pool, run_id, &["seeding"], "seeded").await?;
            println!("{}", json!({"runId": run_id, "state": "seeded"}));
        }
        MigrationCommand::MarkSourceStopped {
            run_id,
            target_profile,
        } => {
            let pool = target_pool(config).await?;
            ensure_target_schema(config, &pool).await?;
            ensure_run_profile(&pool, run_id, target_profile).await?;
            let changed = sqlx::query("UPDATE cvm_migration.runs SET state='source_stopped',needs_reset=TRUE WHERE run_id=$1 AND state='seeded'").bind(run_id).execute(&pool).await?;
            if changed.rows_affected() != 1 {
                bail!("source can only be marked stopped after seed");
            }
            println!("{}", json!({"runId": run_id, "state": "source_stopped"}));
        }
        MigrationCommand::CatchUp {
            run_id,
            source_root,
            batch_rows,
            target_profile,
        } => {
            let pool = target_pool(config).await?;
            ensure_target_schema(config, &pool).await?;
            ensure_run_profile(&pool, run_id, target_profile).await?;
            let manifest = inspect_source(source_root).await?;
            assert_manifest_matches_prepared_run(&pool, run_id, &manifest).await?;
            transition(
                &pool,
                run_id,
                &["source_stopped", "paused", "failed"],
                "catching_up",
            )
            .await?;
            reset_direct_import(&pool, run_id).await?;
            let source_objects = manifest
                .tables
                .iter()
                .map(|table| table.name.clone())
                .collect::<HashSet<_>>();
            clear_missing_source_objects(&pool, run_id, &source_objects).await?;
            register_manifest_tables(
                &pool,
                run_id,
                &manifest.tables,
                &manifest.foreign_keys,
                &manifest.unique_indexes,
            )
            .await?;
            register_manifest_assets(&pool, run_id, &manifest.assets).await?;
            if let Err(error) = stage_and_materialize(&pool, run_id, source_root, *batch_rows).await
            {
                record_failure(&pool, run_id, "source_stopped", &error).await;
                return Err(error);
            }
            transition(&pool, run_id, &["catching_up"], "verified").await?;
            println!("{}", json!({"runId": run_id, "state": "verified"}));
        }
        MigrationCommand::Verify {
            run_id,
            strict,
            target_profile,
        } => {
            let pool = target_pool(config).await?;
            ensure_target_schema(config, &pool).await?;
            ensure_run_profile(&pool, run_id, target_profile).await?;
            let state: String =
                sqlx::query_scalar("SELECT state FROM cvm_migration.runs WHERE run_id=$1")
                    .bind(run_id)
                    .fetch_one(&pool)
                    .await?;
            if state != "seeded" && state != "verified" && state != "finalized" {
                bail!("migration run is not verified: {state}");
            }
            if *strict && state == "seeded" {
                bail!("strict migration verification requires the source-stopped catch-up phase");
            }
            let source_root: Option<String> =
                sqlx::query_scalar("SELECT source_root FROM cvm_migration.runs WHERE run_id=$1")
                    .bind(run_id)
                    .fetch_one(&pool)
                    .await?;
            verify_imported_rows(
                &pool,
                run_id,
                *strict,
                source_root.as_deref().map(Path::new),
                None,
            )
            .await?;
            println!(
                "{}",
                json!({"runId": run_id, "state": state, "verified": true})
            );
        }
        MigrationCommand::Status {
            run_id,
            target_profile,
        } => {
            let pool = target_pool(config).await?;
            ensure_target_schema(config, &pool).await?;
            ensure_run_profile(&pool, run_id, target_profile).await?;
            let row: (String, String, Option<String>, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
                "SELECT state,target_profile,source_manifest_sha256,last_error_code,source_root,resume_state FROM cvm_migration.runs WHERE run_id=$1",
            )
            .bind(run_id)
            .fetch_one(&pool)
            .await?;
            let object_summary: (i64, i64, i64) = sqlx::query_as(
                "SELECT COUNT(*)::bigint, COALESCE(SUM(source_rows), 0)::bigint, COALESCE(SUM(imported_rows), 0)::bigint FROM cvm_migration.objects WHERE run_id=$1",
            )
            .bind(run_id)
            .fetch_one(&pool)
            .await?;
            let batch_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM cvm_migration.batches WHERE run_id=$1")
                    .bind(run_id)
                    .fetch_one(&pool)
                    .await?;
            let asset_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM cvm_migration.assets WHERE run_id=$1")
                    .bind(run_id)
                    .fetch_one(&pool)
                    .await?;
            println!(
                "{}",
                json!({
                    "runId": run_id,
                    "targetProfile": row.1,
                    "state": row.0,
                    "manifestSha256": row.2,
                    "errorCode": row.3,
                    "sourceRoot": row.4,
                    "resumeState": row.5,
                    "objects": object_summary.0,
                    "sourceRows": object_summary.1,
                    "importedRows": object_summary.2,
                    "batches": batch_count,
                    "assets": asset_count,
                })
            );
        }
        MigrationCommand::Resume {
            run_id,
            target_profile,
            batch_rows,
        } => {
            let pool = target_pool(config).await?;
            ensure_target_schema(config, &pool).await?;
            ensure_run_profile(&pool, run_id, target_profile).await?;
            let (state, resume_state, source_root): (String, Option<String>, String) =
                sqlx::query_as(
                    "SELECT state,resume_state,source_root FROM cvm_migration.runs WHERE run_id=$1",
                )
                .bind(run_id)
                .fetch_one(&pool)
                .await?;
            let next_state = resumable_state(&state, resume_state.as_deref())?;
            sqlx::query("UPDATE cvm_migration.runs SET state=$2,resume_state=NULL,last_error_code=NULL,updated_at=now() WHERE run_id=$1")
                .bind(run_id)
            .bind(&next_state)
                .execute(&pool)
                .await?;
            drop(pool);
            let command = match next_state.as_str() {
                "prepared" => MigrationCommand::Seed {
                    target_profile: target_profile.clone(),
                    run_id: run_id.clone(),
                    source_root: PathBuf::from(source_root),
                    batch_rows: *batch_rows,
                },
                "source_stopped" => MigrationCommand::CatchUp {
                    target_profile: target_profile.clone(),
                    run_id: run_id.clone(),
                    source_root: PathBuf::from(source_root),
                    batch_rows: *batch_rows,
                },
                other => bail!("migration run cannot resume into state {other}"),
            };
            Box::pin(run_unlocked(config, &command)).await?;
        }
        MigrationCommand::Finalize {
            run_id,
            target_profile,
        } => {
            let pool = target_pool(config).await?;
            ensure_target_schema(config, &pool).await?;
            ensure_run_profile(&pool, run_id, target_profile).await?;
            transition(&pool, run_id, &["verified"], "finalized").await?;
            println!("{}", json!({"runId": run_id, "state": "finalized"}));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_key_uses_primary_key_when_available() {
        let payload = json!({"id": 42, "name": "alpha"});
        let table = SourceTable {
            name: "sample".to_owned(),
            columns: vec![
                SourceColumn {
                    name: "id".to_owned(),
                    declared_type: "INTEGER".to_owned(),
                },
                SourceColumn {
                    name: "name".to_owned(),
                    declared_type: "TEXT".to_owned(),
                },
            ],
            primary_key: vec!["id".to_owned()],
            row_count: 1,
            kind: "table".to_owned(),
        };
        assert_eq!(source_key(&payload, &table, 7).unwrap(), "{\"id\":42}");
    }

    #[test]
    fn source_key_falls_back_to_stable_row_number() {
        let payload = json!({"name": "alpha"});
        let table = SourceTable {
            name: "sample".to_owned(),
            columns: vec![SourceColumn {
                name: "name".to_owned(),
                declared_type: "TEXT".to_owned(),
            }],
            primary_key: Vec::new(),
            row_count: 1,
            kind: "table".to_owned(),
        };
        assert_eq!(source_key(&payload, &table, 7).unwrap(), "row:7");
    }

    #[test]
    fn target_table_name_is_deterministic_and_identifier_safe() {
        let first = target_table_for_object("pool_upstream_request_attempts");
        let second = target_table_for_object("pool_upstream_request_attempts");
        assert_eq!(first, second);
        assert_eq!(first, "pool_upstream_request_attempts");
        let long_name = "x".repeat(100);
        assert!(target_table_for_object(&long_name).starts_with("source_"));
    }

    #[test]
    fn copy_csv_field_escapes_quotes_and_preserves_newlines() {
        let mut encoded = String::new();
        copy_csv_field("a\"b\nc", &mut encoded);
        assert_eq!(encoded, "\"a\"\"b\nc\"");
    }

    #[test]
    fn source_object_classification_matches_sync_semantics() {
        assert_eq!(classify_source_object("codex_invocations"), "append_only");
        assert_eq!(
            classify_source_object("proxy_raw_payload_blobs"),
            "immutable"
        );
        assert_eq!(classify_source_object("hourly_rollup_stats"), "derived");
        assert_eq!(classify_source_object("proxy_model_settings"), "mutable");
    }

    #[test]
    fn resume_only_reenters_known_migration_phases() {
        assert_eq!(resumable_state("paused", None).unwrap(), "prepared");
        assert_eq!(
            resumable_state("failed", Some("source_stopped")).unwrap(),
            "source_stopped"
        );
        assert!(resumable_state("seeded", None).is_err());
        assert!(resumable_state("failed", Some("verified")).is_err());
    }

    #[test]
    fn source_types_map_without_losing_numeric_or_blob_semantics() {
        assert_eq!(postgres_type_for_source_column("INTEGER"), "BIGINT");
        assert_eq!(postgres_type_for_source_column("REAL"), "DOUBLE PRECISION");
        assert_eq!(postgres_type_for_source_column("NUMERIC(20,6)"), "NUMERIC");
        assert_eq!(postgres_type_for_source_column("BLOB"), "BYTEA");
        assert_eq!(postgres_type_for_source_column("TEXT"), "TEXT");
    }

    #[tokio::test]
    async fn encode_row_preserves_declared_text_and_blob_types() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let row = sqlx::query("SELECT 'alpha' AS text_col, x'00FF' AS blob_col, 7 AS int_col")
            .fetch_one(&pool)
            .await
            .unwrap();
        let table = SourceTable {
            name: "sample".to_owned(),
            columns: vec![
                SourceColumn {
                    name: "text_col".to_owned(),
                    declared_type: "TEXT".to_owned(),
                },
                SourceColumn {
                    name: "blob_col".to_owned(),
                    declared_type: "BLOB".to_owned(),
                },
                SourceColumn {
                    name: "int_col".to_owned(),
                    declared_type: "INTEGER".to_owned(),
                },
            ],
            primary_key: Vec::new(),
            row_count: 1,
            kind: "mutable".to_owned(),
        };
        let payload = encode_row(&row, &table).await.unwrap();
        assert_eq!(payload["text_col"], json!("alpha"));
        assert_eq!(payload["blob_col"], json!("AP8="));
        assert_eq!(payload["int_col"], json!(7));
        pool.close().await;
    }

    #[test]
    fn identifiers_and_literals_are_quoted_for_dynamic_sql() {
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_literal("a'b"), "'a''b'");
    }

    #[test]
    fn manifest_checksum_is_stable_across_json_formatting() {
        let manifest = SourceManifest {
            format_version: 2,
            source_root: "/tmp/source".to_owned(),
            tables: vec![SourceTable {
                name: "accounts".to_owned(),
                columns: vec![SourceColumn {
                    name: "id".to_owned(),
                    declared_type: "INTEGER".to_owned(),
                }],
                primary_key: vec!["id".to_owned()],
                row_count: 1,
                kind: "mutable".to_owned(),
            }],
            foreign_keys: Vec::new(),
            unique_indexes: Vec::new(),
            assets: Vec::new(),
        };
        let compact = serde_json::to_string(&manifest).unwrap();
        let pretty = serde_json::to_string_pretty(&manifest).unwrap();
        let compact_manifest: SourceManifest = serde_json::from_str(&compact).unwrap();
        let pretty_manifest: SourceManifest = serde_json::from_str(&pretty).unwrap();
        assert_eq!(
            manifest_checksum(&compact_manifest).unwrap(),
            manifest_checksum(&pretty_manifest).unwrap()
        );
    }

    #[test]
    fn manifest_checksum_tracks_structure_but_allows_online_data_changes() {
        let mut first = SourceManifest {
            format_version: 2,
            source_root: "/tmp/source".to_owned(),
            tables: vec![SourceTable {
                name: "accounts".to_owned(),
                columns: vec![SourceColumn {
                    name: "id".to_owned(),
                    declared_type: "INTEGER".to_owned(),
                }],
                primary_key: vec!["id".to_owned()],
                row_count: 1,
                kind: "mutable".to_owned(),
            }],
            foreign_keys: Vec::new(),
            unique_indexes: Vec::new(),
            assets: vec![SourceAsset {
                relative_path: "proxy_raw_payloads/a.bin".to_owned(),
                size_bytes: 1,
                sha256: "00".repeat(32),
                asset_kind: "raw_payload".to_owned(),
            }],
        };
        let original = manifest_checksum(&first).unwrap();
        first.tables[0].row_count = 999;
        first.assets[0].sha256 = "11".repeat(32);
        assert_eq!(original, manifest_checksum(&first).unwrap());
        first.tables[0].columns[0].declared_type = "TEXT".to_owned();
        assert_ne!(original, manifest_checksum(&first).unwrap());
    }

    #[tokio::test]
    async fn inspect_source_snapshot_captures_tables_and_binary_assets() {
        let root = std::env::temp_dir().join(format!(
            "cvm-migration-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(root.join("proxy_raw_payloads")).unwrap();
        let database = root.join("codex_vibe_monitor.db");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE sample (id INTEGER PRIMARY KEY, payload BLOB NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE UNIQUE INDEX sample_payload_unique ON sample(payload)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id) ON DELETE CASCADE)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO sample(id,payload) VALUES(1, x'00FF')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        fs::write(
            root.join("proxy_raw_payloads").join("payload.bin"),
            [0_u8, 255_u8],
        )
        .unwrap();

        let manifest = inspect_source(&root).await.unwrap();
        assert_eq!(manifest.format_version, 2);
        assert!(manifest.tables.iter().any(|table| {
            table.name == "sample" && table.row_count == 1 && table.primary_key == vec!["id"]
        }));
        assert!(manifest.foreign_keys.iter().any(|foreign_key| {
            foreign_key.table == "child"
                && foreign_key.column == "parent_id"
                && foreign_key.referenced_table == "parent"
                && foreign_key.referenced_column == "id"
                && foreign_key.on_delete == "CASCADE"
        }));
        assert!(manifest.unique_indexes.iter().any(|index| {
            index.table == "sample"
                && index.name == "sample_payload_unique"
                && index.columns == vec!["payload"]
        }));
        assert_eq!(manifest.assets.len(), 1);
        assert_eq!(
            manifest.assets[0].relative_path,
            "proxy_raw_payloads/payload.bin"
        );
        assert_eq!(manifest.assets[0].size_bytes, 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn online_backup_keeps_a_consistent_binary_snapshot_while_source_writes() {
        let root = std::env::temp_dir().join(format!(
            "cvm-migration-backup-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("codex_vibe_monitor.db");
        let writer = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&database)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&writer)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE sample (id INTEGER PRIMARY KEY, payload BLOB NOT NULL)")
            .execute(&writer)
            .await
            .unwrap();
        for id in 0..32_i64 {
            sqlx::query("INSERT INTO sample(id,payload) VALUES(?1,?2)")
                .bind(id)
                .bind(vec![0_u8, 255_u8, id as u8])
                .execute(&writer)
                .await
                .unwrap();
        }
        let writer_task = tokio::spawn(async move {
            for id in 32..128_i64 {
                sqlx::query("INSERT INTO sample(id,payload) VALUES(?1,?2)")
                    .bind(id)
                    .bind(vec![0_u8, 255_u8, id as u8])
                    .execute(&writer)
                    .await
                    .unwrap();
                tokio::task::yield_now().await;
            }
            writer.close().await;
        });

        let snapshot = create_online_backup(&database).await.unwrap();
        let _snapshot_guard = SnapshotGuard(snapshot.clone());
        writer_task.await.unwrap();
        let snapshot_pool = source_pool(&snapshot).await.unwrap();
        let snapshot_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sample")
            .fetch_one(&snapshot_pool)
            .await
            .unwrap();
        let snapshot_payload: Vec<u8> = sqlx::query_scalar("SELECT payload FROM sample WHERE id=0")
            .fetch_one(&snapshot_pool)
            .await
            .unwrap();
        assert!((32..=128).contains(&snapshot_count));
        assert_eq!(snapshot_payload, vec![0_u8, 255_u8, 0_u8]);
        snapshot_pool.close().await;
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn inspect_source_rejects_unsupported_unique_index_shapes() {
        let root = std::env::temp_dir().join(format!(
            "cvm-migration-index-shapes-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("codex_vibe_monitor.db");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&database)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("CREATE TABLE sample (id INTEGER PRIMARY KEY, name TEXT, active INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE UNIQUE INDEX sample_active_unique ON sample(name) WHERE active = 1")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let manifest = inspect_source(&root).await.unwrap();
        assert_eq!(
            manifest.unique_indexes[0].predicate.as_deref(),
            Some("active = 1")
        );

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&database)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("DROP INDEX sample_active_unique")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE UNIQUE INDEX sample_expression_unique ON sample(lower(name))")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let error = inspect_source(&root).await.unwrap_err().to_string();
        assert!(error.contains("expression"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_order_by_is_deterministic_for_primary_keys_and_rowid_fallback() {
        let keyed = SourceTable {
            name: "sample".to_owned(),
            columns: Vec::new(),
            primary_key: vec!["tenant".to_owned(), "id".to_owned()],
            row_count: 0,
            kind: "mutable".to_owned(),
        };
        let unkeyed = SourceTable {
            primary_key: Vec::new(),
            ..keyed.clone()
        };
        assert_eq!(source_order_by(&keyed), "\"tenant\", \"id\"");
        assert_eq!(source_order_by(&unkeyed), "rowid");
    }

    #[test]
    fn source_asset_inventory_is_deterministic_and_hashes_binary_files() {
        let root = std::env::temp_dir().join(format!("cvm-migration-assets-{}", nanoid::nanoid!()));
        let archive = root.join("archives").join("2026");
        fs::create_dir_all(&archive).unwrap();
        fs::write(archive.join("payload.bin"), [0, 159, 146, 150]).unwrap();
        fs::write(root.join("archives").join("z-last"), b"last").unwrap();

        let assets = collect_source_assets(&root).unwrap();
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].relative_path, "archives/2026/payload.bin");
        assert_eq!(assets[0].size_bytes, 4);
        assert_eq!(assets[0].sha256.len(), 64);
        assert_eq!(assets[1].relative_path, "archives/z-last");
        let _ = fs::remove_dir_all(root);
    }
}
