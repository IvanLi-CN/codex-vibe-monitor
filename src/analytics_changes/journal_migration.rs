use super::*;
use anyhow::ensure;

pub(crate) const ANALYTICS_CHANGE_EVENTS_CREATE_SQL: &str = r#"
    CREATE TABLE IF NOT EXISTS analytics_change_events (
        change_id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_version INTEGER NOT NULL,
        fact_date_semantics_version INTEGER NOT NULL DEFAULT 1,
        source_instance_id TEXT NOT NULL DEFAULT 'legacy-sqlite',
        event_id TEXT NOT NULL DEFAULT '',
        operation_kind TEXT NOT NULL DEFAULT 'legacy',
        fact_semantics_version INTEGER NOT NULL DEFAULT 1,
        stats_semantics_version INTEGER NOT NULL DEFAULT 1,
        payload_digest TEXT NOT NULL DEFAULT '',
        event_kind TEXT NOT NULL CHECK(event_kind IN ('upsert', 'tombstone', 'coverage')),
        entity_kind TEXT NOT NULL,
        source TEXT NOT NULL,
        source_row_id INTEGER,
        entity_key TEXT NOT NULL,
        generation INTEGER NOT NULL,
        coverage_revision INTEGER NOT NULL,
        occurred_at TEXT NOT NULL,
        fact_json TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
    "#;

const JOURNAL_MIGRATION_TABLE: &str = "analytics_change_journal_migrations";
const JOURNAL_MIGRATION_ID: &str = "analytics-change-events-complete-inline-payload-v1";
const JOURNAL_REBUILD_TABLE: &str = "analytics_change_events__journal_payload_v1";

const BASE_COLUMN_NAMES: &[&str] = &[
    "change_id",
    "event_version",
    "event_kind",
    "entity_kind",
    "source",
    "source_row_id",
    "entity_key",
    "generation",
    "coverage_revision",
    "occurred_at",
    "fact_json",
    "created_at",
];

const COMPATIBILITY_COLUMN_NAMES: &[&str] = &[
    "fact_date_semantics_version",
    "source_instance_id",
    "event_id",
    "operation_kind",
    "fact_semantics_version",
    "stats_semantics_version",
    "payload_digest",
];

const CANONICAL_COLUMN_NAMES: &[&str] = &[
    "change_id",
    "event_version",
    "fact_date_semantics_version",
    "source_instance_id",
    "event_id",
    "operation_kind",
    "fact_semantics_version",
    "stats_semantics_version",
    "payload_digest",
    "event_kind",
    "entity_kind",
    "source",
    "source_row_id",
    "entity_key",
    "generation",
    "coverage_revision",
    "occurred_at",
    "fact_json",
    "created_at",
];

const MARKER_COLUMN_NAMES: &[&str] = &[
    "migration_id",
    "source_schema_fingerprint",
    "target_schema_fingerprint",
    "source_sequence",
    "source_row_count",
    "source_max_change_id",
    "manifest_digest",
    "applied_at",
];

const EVENT_ID_INDEX: &str = "idx_analytics_change_events_event_id";
const GENERATION_INDEX: &str = "idx_analytics_change_events_generation_cursor";
const ENTITY_INDEX: &str = "idx_analytics_change_events_entity_cursor";

#[derive(Debug, Clone)]
struct ColumnShape {
    name: String,
    data_type: String,
    not_null: bool,
    primary_key: bool,
}

#[derive(Debug, Clone)]
struct IndexShape {
    name: String,
    unique: bool,
    columns: Vec<String>,
}

#[derive(Debug, Clone)]
struct TableShape {
    columns: Vec<ColumnShape>,
    indexes: Vec<IndexShape>,
    sql: String,
    trigger_count: i64,
    foreign_key_count: i64,
}

#[derive(Debug, Clone)]
struct Manifest {
    row_count: i64,
    max_change_id: i64,
    digest: String,
}

/// Validate the schema before the compatibility additions in `ensure_analytics_changes_schema`.
///
/// The old Journal has a deliberately small set of known column variants. Anything else is
/// rejected before an `ALTER TABLE`, because a forward migration must not guess what an unknown
/// application-owned table means.
pub(crate) async fn validate_preflight(pool: &Pool<Sqlite>) -> Result<()> {
    let mut connection = pool
        .acquire()
        .await
        .context("acquire SQLite connection for analytics journal migration preflight")?;

    if table_exists(&mut connection, JOURNAL_MIGRATION_TABLE).await? {
        validate_marker_table(&mut connection).await?;
    }

    if !table_exists(&mut connection, "analytics_change_events").await? {
        return Ok(());
    }

    let shape = inspect_table(&mut connection, "analytics_change_events").await?;
    validate_table_safety(&shape, "analytics_change_events")?;
    validate_supported_source_shape(&shape)?;
    Ok(())
}

/// Apply or reconcile the complete-inline-payload Journal migration.
///
/// All schema changes and the marker are committed by one SQLite write transaction. If the
/// process loses the response after COMMIT, the next invocation observes the marker and the
/// canonical structure and returns without rebuilding the table.
pub(crate) async fn ensure_complete_inline_payload_storage(pool: &Pool<Sqlite>) -> Result<()> {
    if migration_already_applied(pool).await? {
        return Ok(());
    }

    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("begin analytics Journal payload migration")?;

    let result = migrate_in_transaction(transaction.as_mut()).await;
    match result {
        Ok(()) => transaction
            .commit()
            .await
            .context("commit analytics Journal payload migration"),
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

async fn migration_already_applied(pool: &Pool<Sqlite>) -> Result<bool> {
    let mut connection = pool
        .acquire()
        .await
        .context("acquire SQLite connection for analytics Journal migration recognition")?;
    if !table_exists(&mut connection, JOURNAL_MIGRATION_TABLE).await? {
        return Ok(false);
    }
    validate_marker_table(&mut connection).await?;
    let Some(marker) = load_marker(&mut connection).await? else {
        return Ok(false);
    };
    ensure!(
        table_exists(&mut connection, "analytics_change_events").await?,
        "analytics Journal migration marker exists without the events table"
    );
    let shape = inspect_table(&mut connection, "analytics_change_events").await?;
    validate_table_safety(&shape, "analytics_change_events")?;
    ensure_canonical_shape(&shape)?;
    ensure_marker_matches_shape(&marker, &shape)?;
    ensure_rebuild_table_absent(&mut connection).await?;
    Ok(true)
}

async fn migrate_in_transaction(connection: &mut SqliteConnection) -> Result<()> {
    ensure_marker_table(connection).await?;

    if let Some(marker) = load_marker(connection).await? {
        let shape = inspect_table(connection, "analytics_change_events").await?;
        validate_table_safety(&shape, "analytics_change_events")?;
        ensure_canonical_shape(&shape)?;
        ensure_marker_matches_shape(&marker, &shape)?;
        ensure_rebuild_table_absent(connection).await?;
        return Ok(());
    }

    ensure_rebuild_table_absent(connection).await?;

    if !table_exists(connection, "analytics_change_events").await? {
        sqlx::query(ANALYTICS_CHANGE_EVENTS_CREATE_SQL)
            .execute(&mut *connection)
            .await
            .context("create canonical analytics change event table")?;
        create_indexes(connection).await?;
        let shape = inspect_table(connection, "analytics_change_events").await?;
        ensure_canonical_shape(&shape)?;
        let manifest = table_manifest(connection, "analytics_change_events").await?;
        let sequence = read_sequence(connection, "analytics_change_events").await?;
        insert_marker(connection, "fresh-create", &shape, sequence, &manifest).await?;
        return Ok(());
    }

    let source_shape = inspect_table(connection, "analytics_change_events").await?;
    validate_table_safety(&source_shape, "analytics_change_events")?;
    if is_canonical_shape(&source_shape) {
        let manifest = table_manifest(connection, "analytics_change_events").await?;
        let sequence = read_sequence(connection, "analytics_change_events").await?;
        insert_marker(
            connection,
            &schema_fingerprint(&source_shape),
            &source_shape,
            sequence,
            &manifest,
        )
        .await?;
        return Ok(());
    }

    validate_supported_source_shape(&source_shape)?;
    let source_fingerprint = schema_fingerprint(&source_shape);
    let source_sequence = read_sequence(connection, "analytics_change_events").await?;
    let source_manifest = table_manifest(connection, "analytics_change_events").await?;

    sqlx::query(&format!(
        "CREATE TABLE {JOURNAL_REBUILD_TABLE} (
            change_id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_version INTEGER NOT NULL,
            fact_date_semantics_version INTEGER NOT NULL DEFAULT 1,
            source_instance_id TEXT NOT NULL DEFAULT 'legacy-sqlite',
            event_id TEXT NOT NULL DEFAULT '',
            operation_kind TEXT NOT NULL DEFAULT 'legacy',
            fact_semantics_version INTEGER NOT NULL DEFAULT 1,
            stats_semantics_version INTEGER NOT NULL DEFAULT 1,
            payload_digest TEXT NOT NULL DEFAULT '',
            event_kind TEXT NOT NULL CHECK(event_kind IN ('upsert', 'tombstone', 'coverage')),
            entity_kind TEXT NOT NULL,
            source TEXT NOT NULL,
            source_row_id INTEGER,
            entity_key TEXT NOT NULL,
            generation INTEGER NOT NULL,
            coverage_revision INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            fact_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"
    ))
    .execute(&mut *connection)
    .await
    .context("create canonical analytics Journal rebuild table")?;

    sqlx::query(&format!(
        "INSERT INTO {JOURNAL_REBUILD_TABLE} (
            change_id, event_version, fact_date_semantics_version,
            source_instance_id, event_id, operation_kind, fact_semantics_version,
            stats_semantics_version, payload_digest, event_kind, entity_kind,
            source, source_row_id, entity_key, generation, coverage_revision,
            occurred_at, fact_json, created_at
        ) SELECT
            change_id, event_version, fact_date_semantics_version,
            source_instance_id, event_id, operation_kind, fact_semantics_version,
            stats_semantics_version, payload_digest, event_kind, entity_kind,
            source, source_row_id, entity_key, generation, coverage_revision,
            occurred_at, fact_json, created_at
        FROM analytics_change_events
        ORDER BY change_id"
    ))
    .execute(&mut *connection)
    .await
    .context("copy analytics Journal rows into canonical storage")?;

    sqlx::query("DROP TABLE analytics_change_events")
        .execute(&mut *connection)
        .await
        .context("drop old analytics Journal table after copy")?;
    sqlx::query(&format!(
        "ALTER TABLE {JOURNAL_REBUILD_TABLE} RENAME TO analytics_change_events"
    ))
    .execute(&mut *connection)
    .await
    .context("rename canonical analytics Journal table")?;
    create_indexes(connection).await?;

    let target_max = max_change_id(connection, "analytics_change_events").await?;
    restore_sequence(
        connection,
        "analytics_change_events",
        source_sequence.max(target_max),
    )
    .await?;

    let target_shape = inspect_table(connection, "analytics_change_events").await?;
    ensure_canonical_shape(&target_shape)?;
    let target_manifest = table_manifest(connection, "analytics_change_events").await?;
    ensure!(
        source_manifest.row_count == target_manifest.row_count
            && source_manifest.max_change_id == target_manifest.max_change_id
            && source_manifest.digest == target_manifest.digest,
        "analytics Journal copy manifest changed during forward migration"
    );
    let target_sequence = read_sequence(connection, "analytics_change_events").await?;
    ensure!(
        target_sequence >= source_sequence,
        "analytics Journal sqlite_sequence high-water mark regressed"
    );

    insert_marker(
        connection,
        &source_fingerprint,
        &target_shape,
        target_sequence,
        &target_manifest,
    )
    .await
}

async fn ensure_marker_table(connection: &mut SqliteConnection) -> Result<()> {
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {JOURNAL_MIGRATION_TABLE} (
            migration_id TEXT PRIMARY KEY,
            source_schema_fingerprint TEXT NOT NULL,
            target_schema_fingerprint TEXT NOT NULL,
            source_sequence INTEGER NOT NULL CHECK(source_sequence >= 0),
            source_row_count INTEGER NOT NULL CHECK(source_row_count >= 0),
            source_max_change_id INTEGER NOT NULL CHECK(source_max_change_id >= 0),
            manifest_digest TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"
    ))
    .execute(&mut *connection)
    .await
    .context("ensure analytics Journal migration marker table")?;
    validate_marker_table(connection).await
}

async fn validate_marker_table(connection: &mut SqliteConnection) -> Result<()> {
    let shape = inspect_table(connection, JOURNAL_MIGRATION_TABLE).await?;
    let names = shape
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();
    ensure!(
        names == MARKER_COLUMN_NAMES,
        "unsupported analytics Journal migration marker schema"
    );
    for column in &shape.columns {
        let expected = match column.name.as_str() {
            "migration_id" => ("TEXT", false, true),
            "source_schema_fingerprint"
            | "target_schema_fingerprint"
            | "manifest_digest"
            | "applied_at" => ("TEXT", true, false),
            "source_sequence" | "source_row_count" | "source_max_change_id" => {
                ("INTEGER", true, false)
            }
            _ => unreachable!("marker column names were validated above"),
        };
        ensure_column_shape(column, expected, JOURNAL_MIGRATION_TABLE)?;
    }
    ensure!(
        shape.trigger_count == 0 && shape.foreign_key_count == 0,
        "analytics Journal migration marker has unsupported triggers or foreign keys"
    );
    Ok(())
}

async fn load_marker(connection: &mut SqliteConnection) -> Result<Option<Marker>> {
    sqlx::query_as::<_, Marker>(&format!(
        "SELECT migration_id, source_schema_fingerprint, target_schema_fingerprint,
                source_sequence, source_row_count, source_max_change_id, manifest_digest,
                applied_at
         FROM {JOURNAL_MIGRATION_TABLE}
         WHERE migration_id = ?1"
    ))
    .bind(JOURNAL_MIGRATION_ID)
    .fetch_optional(&mut *connection)
    .await
    .context("load analytics Journal migration marker")
}

#[derive(Debug, FromRow)]
struct Marker {
    migration_id: String,
    source_schema_fingerprint: String,
    target_schema_fingerprint: String,
    source_sequence: i64,
    source_row_count: i64,
    source_max_change_id: i64,
    manifest_digest: String,
    applied_at: String,
}

async fn insert_marker(
    connection: &mut SqliteConnection,
    source_fingerprint: &str,
    target_shape: &TableShape,
    source_sequence: i64,
    manifest: &Manifest,
) -> Result<()> {
    sqlx::query(&format!(
        "INSERT INTO {JOURNAL_MIGRATION_TABLE} (
            migration_id, source_schema_fingerprint, target_schema_fingerprint,
            source_sequence, source_row_count, source_max_change_id, manifest_digest
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
    ))
    .bind(JOURNAL_MIGRATION_ID)
    .bind(source_fingerprint)
    .bind(schema_fingerprint(target_shape))
    .bind(source_sequence)
    .bind(manifest.row_count)
    .bind(manifest.max_change_id)
    .bind(&manifest.digest)
    .execute(&mut *connection)
    .await
    .context("record analytics Journal migration marker")?;
    Ok(())
}

fn ensure_marker_matches_shape(marker: &Marker, shape: &TableShape) -> Result<()> {
    ensure!(
        marker.migration_id == JOURNAL_MIGRATION_ID,
        "unknown analytics Journal migration marker"
    );
    ensure!(
        marker.target_schema_fingerprint == schema_fingerprint(shape),
        "analytics Journal migration marker does not match the current table structure"
    );
    ensure!(
        marker.source_sequence >= 0
            && marker.source_row_count >= 0
            && marker.source_max_change_id >= 0
            && !marker.manifest_digest.is_empty()
            && !marker.applied_at.is_empty(),
        "analytics Journal migration marker is incomplete"
    );
    Ok(())
}

async fn ensure_rebuild_table_absent(connection: &mut SqliteConnection) -> Result<()> {
    ensure!(
        !table_exists(connection, JOURNAL_REBUILD_TABLE).await?,
        "leftover analytics Journal rebuild table requires operator inspection"
    );
    Ok(())
}

async fn table_exists(connection: &mut SqliteConnection, table_name: &str) -> Result<bool> {
    sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
        )",
    )
    .bind(table_name)
    .fetch_one(&mut *connection)
    .await
    .with_context(|| format!("inspect SQLite table {table_name}"))
    .map(|value| value != 0)
}

async fn inspect_table(connection: &mut SqliteConnection, table_name: &str) -> Result<TableShape> {
    let pragma_name = table_name.replace('"', "\"\"");
    let table_info_sql = format!("PRAGMA table_info(\"{pragma_name}\")");
    let columns = sqlx::query(&table_info_sql)
        .fetch_all(&mut *connection)
        .await
        .with_context(|| format!("inspect SQLite columns for {table_name}"))?
        .into_iter()
        .map(|row| {
            Ok(ColumnShape {
                name: row.try_get("name")?,
                data_type: row.try_get::<String, _>("type")?.to_ascii_uppercase(),
                not_null: row.try_get::<i64, _>("notnull")? != 0,
                primary_key: row.try_get::<i64, _>("pk")? != 0,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let index_list_sql = format!("PRAGMA index_list(\"{pragma_name}\")");
    let index_rows = sqlx::query(&index_list_sql)
        .fetch_all(&mut *connection)
        .await
        .with_context(|| format!("inspect SQLite indexes for {table_name}"))?;
    let mut indexes = Vec::with_capacity(index_rows.len());
    for row in index_rows {
        let name: String = row.try_get("name")?;
        let unique = row.try_get::<i64, _>("unique")? != 0;
        let index_name = name.replace('"', "\"\"");
        let index_info_sql = format!("PRAGMA index_info(\"{index_name}\")");
        let columns = sqlx::query(&index_info_sql)
            .fetch_all(&mut *connection)
            .await
            .with_context(|| format!("inspect SQLite index {name} for {table_name}"))?
            .into_iter()
            .map(|row| row.try_get::<String, _>("name"))
            .collect::<std::result::Result<Vec<_>, sqlx::Error>>()?;
        indexes.push(IndexShape {
            name,
            unique,
            columns,
        });
    }

    let sql = sqlx::query_scalar::<_, Option<String>>(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
    )
    .bind(table_name)
    .fetch_one(&mut *connection)
    .await
    .with_context(|| format!("load SQLite table definition for {table_name}"))?
    .unwrap_or_default();
    let trigger_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND tbl_name = ?1",
    )
    .bind(table_name)
    .fetch_one(&mut *connection)
    .await
    .with_context(|| format!("inspect SQLite triggers for {table_name}"))?;
    let foreign_key_pragma = format!("PRAGMA foreign_key_list(\"{pragma_name}\")");
    let foreign_key_count = i64::try_from(
        sqlx::query(&foreign_key_pragma)
            .fetch_all(&mut *connection)
            .await
            .with_context(|| format!("inspect SQLite foreign keys for {table_name}"))?
            .len(),
    )?;

    Ok(TableShape {
        columns,
        indexes,
        sql,
        trigger_count,
        foreign_key_count,
    })
}

fn validate_table_safety(shape: &TableShape, table_name: &str) -> Result<()> {
    ensure!(
        shape.trigger_count == 0 && shape.foreign_key_count == 0,
        "unsupported triggers or foreign keys reference analytics Journal table {table_name}"
    );
    for index in &shape.indexes {
        let expected = match index.name.as_str() {
            EVENT_ID_INDEX => (&["event_id"][..], true),
            GENERATION_INDEX => (&["generation", "change_id"][..], false),
            ENTITY_INDEX => (&["entity_kind", "source_row_id", "change_id"][..], false),
            _ => bail!(
                "unsupported index {} on analytics Journal table",
                index.name
            ),
        };
        ensure!(
            index.unique == expected.1 && index.columns == expected.0,
            "unsupported definition for analytics Journal index {}",
            index.name
        );
    }
    Ok(())
}

fn validate_supported_source_shape(shape: &TableShape) -> Result<()> {
    let names = shape
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();
    let has_base = BASE_COLUMN_NAMES.iter().all(|name| names.contains(name));
    let only_supported = names
        .iter()
        .all(|name| BASE_COLUMN_NAMES.contains(name) || COMPATIBILITY_COLUMN_NAMES.contains(name));
    ensure!(
        has_base && only_supported,
        "unsupported analytics Journal schema; refusing blind forward migration"
    );
    for column in &shape.columns {
        let expected = expected_event_column(&column.name)
            .context("analytics Journal schema contains an unknown column")?;
        ensure_column_shape(column, expected, "analytics_change_events")?;
    }
    let normalized_sql = normalize_sql(&shape.sql);
    ensure!(
        normalized_sql.contains("check(event_kindin('upsert','tombstone','coverage'))"),
        "unsupported analytics Journal event_kind constraint"
    );
    let has_fact_length_check = normalized_sql.contains("check(length(fact_json)");
    ensure!(
        !has_fact_length_check || normalized_sql.contains("check(length(fact_json)<=65536)"),
        "unsupported analytics Journal fact_json length constraint"
    );
    let fact_json = shape
        .columns
        .iter()
        .find(|column| column.name == "fact_json")
        .context("analytics Journal schema is missing fact_json")?;
    ensure!(
        fact_json.data_type == "TEXT" && fact_json.not_null,
        "unsupported analytics Journal fact_json storage type"
    );
    let change_id = shape
        .columns
        .iter()
        .find(|column| column.name == "change_id")
        .context("analytics Journal schema is missing change_id")?;
    ensure!(
        change_id.data_type == "INTEGER" && change_id.primary_key,
        "unsupported analytics Journal change_id definition"
    );
    ensure!(
        normalize_sql(&shape.sql).contains("autoincrement"),
        "analytics Journal change_id must use AUTOINCREMENT"
    );
    Ok(())
}

fn ensure_canonical_shape(shape: &TableShape) -> Result<()> {
    ensure!(
        is_canonical_shape(shape),
        "analytics Journal table is not the complete-inline-payload canonical schema"
    );
    Ok(())
}

fn is_canonical_shape(shape: &TableShape) -> bool {
    let fact_json_is_unbounded_text = shape
        .columns
        .iter()
        .find(|column| column.name == "fact_json")
        .is_some_and(|column| column.data_type == "TEXT" && column.not_null);
    let indexes_match = [
        (EVENT_ID_INDEX, true, &["event_id"][..]),
        (GENERATION_INDEX, false, &["generation", "change_id"][..]),
        (
            ENTITY_INDEX,
            false,
            &["entity_kind", "source_row_id", "change_id"][..],
        ),
    ]
    .iter()
    .all(|(name, unique, columns)| {
        shape.indexes.iter().any(|index| {
            index.name == *name
                && index.unique == *unique
                && index
                    .columns
                    .iter()
                    .map(String::as_str)
                    .eq(columns.iter().copied())
        })
    });
    shape
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .eq(CANONICAL_COLUMN_NAMES.iter().copied())
        && canonical_columns_match(shape)
        && fact_json_is_unbounded_text
        && normalize_sql(&shape.sql)
            .contains("check(event_kindin('upsert','tombstone','coverage'))")
        && !normalize_sql(&shape.sql).contains("check(length(fact_json)<=65536)")
        && indexes_match
}

fn canonical_columns_match(shape: &TableShape) -> bool {
    shape.columns.len() == CANONICAL_COLUMN_NAMES.len()
        && shape.columns.iter().all(|column| {
            expected_event_column(&column.name)
                .is_some_and(|expected| column_shape_matches(column, expected))
        })
}

fn expected_event_column(name: &str) -> Option<(&'static str, bool, bool)> {
    Some(match name {
        "change_id" => ("INTEGER", false, true),
        "event_version"
        | "fact_date_semantics_version"
        | "fact_semantics_version"
        | "stats_semantics_version"
        | "generation"
        | "coverage_revision" => ("INTEGER", true, false),
        "source_row_id" => ("INTEGER", false, false),
        "source_instance_id" | "event_id" | "operation_kind" | "payload_digest" | "event_kind"
        | "entity_kind" | "source" | "entity_key" | "occurred_at" | "fact_json" | "created_at" => {
            ("TEXT", true, false)
        }
        _ => return None,
    })
}

fn ensure_column_shape(
    column: &ColumnShape,
    expected: (&str, bool, bool),
    table_name: &str,
) -> Result<()> {
    ensure!(
        column_shape_matches(column, expected),
        "unsupported {table_name} column {} definition",
        column.name
    );
    Ok(())
}

fn column_shape_matches(column: &ColumnShape, expected: (&str, bool, bool)) -> bool {
    column.data_type == expected.0
        && column.not_null == expected.1
        && column.primary_key == expected.2
}

fn schema_fingerprint(shape: &TableShape) -> String {
    let mut hasher = Sha256::new();
    for column in &shape.columns {
        hasher.update(column.name.as_bytes());
        hasher.update([0]);
        hasher.update(column.data_type.as_bytes());
        hasher.update([0]);
        hasher.update([u8::from(column.not_null), u8::from(column.primary_key)]);
        hasher.update([0xff]);
    }
    let mut indexes = shape.indexes.clone();
    indexes.sort_by(|left, right| left.name.cmp(&right.name));
    for index in indexes {
        hasher.update(index.name.as_bytes());
        hasher.update([u8::from(index.unique), 0]);
        for column in &index.columns {
            hasher.update(column.as_bytes());
            hasher.update([0]);
        }
        hasher.update([0xff]);
    }
    hasher.update([u8::from(
        normalize_sql(&shape.sql).contains("check(event_kindin('upsert','tombstone','coverage'))"),
    )]);
    hasher.update([u8::from(
        normalize_sql(&shape.sql).contains("check(length(fact_json)<=65536)"),
    )]);
    format!("sha256:{:x}", hasher.finalize())
}

fn normalize_sql(sql: &str) -> String {
    sql.to_ascii_lowercase()
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect()
}

async fn create_indexes(connection: &mut SqliteConnection) -> Result<()> {
    for statement in [
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_analytics_change_events_event_id ON analytics_change_events (event_id)",
        "CREATE INDEX IF NOT EXISTS idx_analytics_change_events_generation_cursor ON analytics_change_events (generation, change_id)",
        "CREATE INDEX IF NOT EXISTS idx_analytics_change_events_entity_cursor ON analytics_change_events (entity_kind, source_row_id, change_id)",
    ] {
        sqlx::query(statement)
            .execute(&mut *connection)
            .await
            .with_context(|| format!("ensure analytics Journal index: {statement}"))?;
    }
    Ok(())
}

async fn read_sequence(connection: &mut SqliteConnection, table_name: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(seq), 0) FROM sqlite_sequence WHERE name = ?1",
    )
    .bind(table_name)
    .fetch_one(&mut *connection)
    .await
    .with_context(|| format!("read SQLite sequence for {table_name}"))
}

async fn restore_sequence(
    connection: &mut SqliteConnection,
    table_name: &str,
    sequence: i64,
) -> Result<()> {
    sqlx::query("UPDATE sqlite_sequence SET seq = MAX(seq, ?2) WHERE name = ?1")
        .bind(table_name)
        .bind(sequence)
        .execute(&mut *connection)
        .await
        .context("restore analytics Journal sqlite_sequence high-water mark")?;

    let sequence_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name = ?1)",
    )
    .bind(table_name)
    .fetch_one(&mut *connection)
    .await
    .context("check restored analytics Journal sqlite_sequence high-water mark")?
        != 0;
    if !sequence_exists {
        sqlx::query("INSERT INTO sqlite_sequence (name, seq) VALUES (?1, ?2)")
            .bind(table_name)
            .bind(sequence)
            .execute(&mut *connection)
            .await
            .context("insert analytics Journal sqlite_sequence high-water mark")?;
    }
    Ok(())
}

async fn max_change_id(connection: &mut SqliteConnection, table_name: &str) -> Result<i64> {
    sqlx::query_scalar::<_, Option<i64>>(&format!("SELECT MAX(change_id) FROM {table_name}"))
        .fetch_one(&mut *connection)
        .await
        .context("load analytics Journal maximum change_id")
        .map(|value| value.unwrap_or(0))
}

async fn table_manifest(connection: &mut SqliteConnection, table_name: &str) -> Result<Manifest> {
    let rows_sql = format!(
        "SELECT change_id, event_version, fact_date_semantics_version,
                CAST(source_instance_id AS BLOB) AS source_instance_id_bytes,
                CAST(event_id AS BLOB) AS event_id_bytes,
                CAST(operation_kind AS BLOB) AS operation_kind_bytes,
                fact_semantics_version, stats_semantics_version,
                CAST(payload_digest AS BLOB) AS payload_digest_bytes,
                CAST(event_kind AS BLOB) AS event_kind_bytes,
                CAST(entity_kind AS BLOB) AS entity_kind_bytes,
                CAST(source AS BLOB) AS source_bytes, source_row_id,
                CAST(entity_key AS BLOB) AS entity_key_bytes,
                generation, coverage_revision,
                CAST(occurred_at AS BLOB) AS occurred_at_bytes,
                CAST(fact_json AS BLOB) AS fact_json_bytes,
                CAST(created_at AS BLOB) AS created_at_bytes
         FROM {table_name}
         ORDER BY change_id"
    );
    let mut rows = sqlx::query(&rows_sql).fetch(&mut *connection);
    let mut hasher = Sha256::new();
    hasher.update(b"analytics-change-events-manifest-v1\0");
    let mut row_count = 0_i64;
    let mut max_id = 0_i64;
    while let Some(row) = rows.try_next().await? {
        let change_id: i64 = row.try_get("change_id")?;
        max_id = max_id.max(change_id);
        row_count = row_count
            .checked_add(1)
            .context("analytics Journal manifest row count overflow")?;
        hash_i64(&mut hasher, change_id);
        hash_i64(&mut hasher, row.try_get("event_version")?);
        hash_i64(&mut hasher, row.try_get("fact_date_semantics_version")?);
        for column in [
            "source_instance_id_bytes",
            "event_id_bytes",
            "operation_kind_bytes",
        ] {
            hash_bytes(&mut hasher, row.try_get::<Vec<u8>, _>(column)?);
        }
        hash_i64(&mut hasher, row.try_get("fact_semantics_version")?);
        hash_i64(&mut hasher, row.try_get("stats_semantics_version")?);
        hash_bytes(
            &mut hasher,
            row.try_get::<Vec<u8>, _>("payload_digest_bytes")?,
        );
        for column in ["event_kind_bytes", "entity_kind_bytes", "source_bytes"] {
            hash_bytes(&mut hasher, row.try_get::<Vec<u8>, _>(column)?);
        }
        match row.try_get::<Option<i64>, _>("source_row_id")? {
            Some(value) => {
                hasher.update([1]);
                hash_i64(&mut hasher, value);
            }
            None => hasher.update([0]),
        }
        hash_bytes(&mut hasher, row.try_get::<Vec<u8>, _>("entity_key_bytes")?);
        hash_i64(&mut hasher, row.try_get("generation")?);
        hash_i64(&mut hasher, row.try_get("coverage_revision")?);
        for column in ["occurred_at_bytes", "fact_json_bytes", "created_at_bytes"] {
            hash_bytes(&mut hasher, row.try_get::<Vec<u8>, _>(column)?);
        }
    }
    Ok(Manifest {
        row_count,
        max_change_id: max_id,
        digest: format!("sha256:{:x}", hasher.finalize()),
    })
}

fn hash_i64(hasher: &mut Sha256, value: i64) {
    hasher.update(value.to_le_bytes());
}

fn hash_bytes(hasher: &mut Sha256, value: Vec<u8>) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}
