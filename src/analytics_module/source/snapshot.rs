use anyhow::{Context, Result, anyhow, bail};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteConnection};

const BUILD_ID_MAX_CHARS: usize = 128;
const MANIFEST_DIGEST_MAX_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceSnapshot {
    pub(crate) build_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) generation: i64,
    pub(crate) high_watermark_change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) retained_from_change_id: i64,
    pub(crate) fact_semantics_version: i64,
    pub(crate) stats_semantics_version: i64,
    pub(crate) manifest_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceRetentionPin {
    pub(crate) pin_id: String,
    pub(crate) build_id: String,
    pub(crate) source_kind: String,
    pub(crate) source_identity: String,
    pub(crate) watermark_change_id: i64,
    pub(crate) expires_at: String,
}

#[derive(Debug, FromRow)]
struct SourceJournalStateRow {
    generation: i64,
    retained_from_change_id: i64,
    capacity: i64,
    invalidation_pending: i64,
    event_count: i64,
}

#[derive(Debug, FromRow)]
struct ExistingRetentionPinRow {
    state: String,
    unexpired: i64,
}

pub(crate) async fn ensure_source_snapshot_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS analytics_source_snapshots (
            build_id TEXT PRIMARY KEY,
            source_instance_id TEXT NOT NULL,
            generation INTEGER NOT NULL,
            high_watermark_change_id INTEGER NOT NULL CHECK(high_watermark_change_id >= 0),
            coverage_revision INTEGER NOT NULL CHECK(coverage_revision >= 0),
            retained_from_change_id INTEGER NOT NULL CHECK(retained_from_change_id >= 0),
            fact_semantics_version INTEGER NOT NULL,
            stats_semantics_version INTEGER NOT NULL,
            manifest_digest TEXT,
            state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active', 'completed', 'failed', 'expired')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("ensure analytics source snapshot table")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS analytics_source_retention_pins (
            pin_id TEXT PRIMARY KEY,
            build_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_identity TEXT NOT NULL,
            watermark_change_id INTEGER NOT NULL CHECK(watermark_change_id >= 0),
            expires_at TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active', 'released', 'expired')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY(build_id) REFERENCES analytics_source_snapshots(build_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("ensure analytics source retention pin table")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_analytics_source_retention_pins_watermark \
         ON analytics_source_retention_pins (state, watermark_change_id, expires_at)",
    )
    .execute(pool)
    .await
    .context("ensure analytics source retention pin index")?;
    Ok(())
}

pub(crate) async fn create_source_snapshot_tx(
    connection: &mut SqliteConnection,
    snapshot: &SourceSnapshot,
) -> Result<()> {
    validate_snapshot(snapshot)?;
    sqlx::query(
        "INSERT INTO analytics_source_snapshots (build_id, source_instance_id, generation, \
         high_watermark_change_id, coverage_revision, retained_from_change_id, \
         fact_semantics_version, stats_semantics_version, manifest_digest) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&snapshot.build_id)
    .bind(&snapshot.source_instance_id)
    .bind(snapshot.generation)
    .bind(snapshot.high_watermark_change_id)
    .bind(snapshot.coverage_revision)
    .bind(snapshot.retained_from_change_id)
    .bind(snapshot.fact_semantics_version)
    .bind(snapshot.stats_semantics_version)
    .bind(&snapshot.manifest_digest)
    .execute(&mut *connection)
    .await
    .context("create analytics source snapshot")?;
    Ok(())
}

pub(crate) async fn create_source_snapshot(
    pool: &Pool<Sqlite>,
    snapshot: &SourceSnapshot,
) -> Result<()> {
    let mut tx = pool.begin().await.context("begin source snapshot")?;
    create_source_snapshot_tx(tx.as_mut(), snapshot).await?;
    tx.commit().await.context("commit source snapshot")?;
    Ok(())
}

pub(crate) async fn load_source_snapshot(
    pool: &Pool<Sqlite>,
    build_id: &str,
) -> Result<Option<SourceSnapshot>> {
    let row = sqlx::query_as::<_, SourceSnapshotRow>(
        "SELECT build_id, source_instance_id, generation, high_watermark_change_id, \
         coverage_revision, retained_from_change_id, fact_semantics_version, \
         stats_semantics_version, manifest_digest FROM analytics_source_snapshots \
         WHERE build_id = ?1 AND state IN ('active', 'completed')",
    )
    .bind(build_id)
    .fetch_optional(pool)
    .await
    .context("load analytics source snapshot")?;
    row.map(SourceSnapshot::try_from).transpose()
}

pub(crate) async fn renew_source_retention_pin_tx(
    connection: &mut SqliteConnection,
    pin: &SourceRetentionPin,
) -> Result<()> {
    validate_pin(pin)?;
    let existing = sqlx::query_as::<_, ExistingRetentionPinRow>(
        "SELECT state, julianday(expires_at) > julianday('now') AS unexpired \
         FROM analytics_source_retention_pins WHERE pin_id = ?1",
    )
    .bind(&pin.pin_id)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(existing) = &existing
        && (existing.state != "active" || existing.unexpired == 0)
    {
        bail!("cannot renew an expired or released source retention pin");
    }
    let snapshot = sqlx::query_as::<_, SourceSnapshotRow>(
        "SELECT build_id, source_instance_id, generation, high_watermark_change_id, \
         coverage_revision, retained_from_change_id, fact_semantics_version, \
         stats_semantics_version, manifest_digest FROM analytics_source_snapshots \
         WHERE build_id = ?1 AND state IN ('active', 'completed')",
    )
    .bind(&pin.build_id)
    .fetch_optional(&mut *connection)
    .await?;
    let snapshot = snapshot
        .map(SourceSnapshot::try_from)
        .transpose()?
        .ok_or_else(|| anyhow!("cannot pin a missing or inactive analytics source snapshot"))?;
    let journal_state = sqlx::query_as::<_, SourceJournalStateRow>(
        "SELECT generation, retained_from_change_id, capacity, invalidation_pending, \
         (SELECT COUNT(*) FROM analytics_change_events) AS event_count \
         FROM analytics_change_state WHERE id = 1",
    )
    .fetch_one(&mut *connection)
    .await
    .context("load analytics source journal state")?;
    validate_snapshot_against_journal(&snapshot, &journal_state)?;
    if pin.watermark_change_id < snapshot.high_watermark_change_id {
        bail!("source retention pin does not cover the snapshot watermark");
    }
    let expires_in_future = sqlx::query_scalar::<_, i64>("SELECT julianday(?1) > julianday('now')")
        .bind(&pin.expires_at)
        .fetch_one(&mut *connection)
        .await?;
    if expires_in_future == 0 {
        bail!("source retention pin expiry must be in the future");
    }
    if let Some(existing) = existing {
        if existing.state != "active" || existing.unexpired == 0 {
            bail!("cannot renew an expired or released source retention pin");
        }
        let updated = sqlx::query(
            "UPDATE analytics_source_retention_pins SET build_id = ?2, source_kind = ?3, \
             source_identity = ?4, watermark_change_id = ?5, expires_at = ?6, \
             updated_at = datetime('now') WHERE pin_id = ?1 AND state = 'active' \
             AND julianday(expires_at) > julianday('now')",
        )
        .bind(&pin.pin_id)
        .bind(&pin.build_id)
        .bind(&pin.source_kind)
        .bind(&pin.source_identity)
        .bind(pin.watermark_change_id)
        .bind(&pin.expires_at)
        .execute(&mut *connection)
        .await
        .context("renew analytics source retention pin")?;
        if updated.rows_affected() != 1 {
            bail!("cannot renew an expired or released source retention pin");
        }
    } else {
        sqlx::query(
            "INSERT INTO analytics_source_retention_pins \
             (pin_id, build_id, source_kind, source_identity, watermark_change_id, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&pin.pin_id)
        .bind(&pin.build_id)
        .bind(&pin.source_kind)
        .bind(&pin.source_identity)
        .bind(pin.watermark_change_id)
        .bind(&pin.expires_at)
        .execute(&mut *connection)
        .await
        .context("create analytics source retention pin")?;
    }
    Ok(())
}

pub(crate) async fn renew_source_retention_pin(
    pool: &Pool<Sqlite>,
    pin: &SourceRetentionPin,
) -> Result<()> {
    let mut tx = pool.begin().await.context("begin source retention pin")?;
    renew_source_retention_pin_tx(tx.as_mut(), pin).await?;
    tx.commit().await.context("commit source retention pin")?;
    Ok(())
}

pub(crate) async fn release_source_retention_pin_tx(
    connection: &mut SqliteConnection,
    pin_id: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE analytics_source_retention_pins SET state = 'released', updated_at = datetime('now') \
         WHERE pin_id = ?1 AND state = 'active'",
    )
    .bind(pin_id)
    .execute(&mut *connection)
    .await
    .context("release analytics source retention pin")?;
    Ok(result.rows_affected() == 1)
}

pub(crate) async fn release_source_retention_pin(
    pool: &Pool<Sqlite>,
    pin_id: &str,
) -> Result<bool> {
    let mut tx = pool.begin().await.context("begin source pin release")?;
    let released = release_source_retention_pin_tx(tx.as_mut(), pin_id).await?;
    tx.commit().await.context("commit source pin release")?;
    Ok(released)
}

pub(crate) async fn assert_source_snapshot_usable(
    pool: &Pool<Sqlite>,
    build_id: &str,
    source_instance_id: &str,
    generation: i64,
    required_through_change_id: i64,
) -> Result<SourceSnapshot> {
    let snapshot = load_source_snapshot(pool, build_id)
        .await?
        .ok_or_else(|| anyhow!("analytics source snapshot is unavailable"))?;
    if snapshot.source_instance_id != source_instance_id {
        bail!("analytics source snapshot belongs to another source instance");
    }
    if snapshot.generation != generation {
        bail!("analytics source snapshot belongs to another generation");
    }
    let journal_state = sqlx::query_as::<_, SourceJournalStateRow>(
        "SELECT generation, retained_from_change_id, capacity, invalidation_pending, \
         (SELECT COUNT(*) FROM analytics_change_events) AS event_count \
         FROM analytics_change_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await
    .context("load analytics source journal state")?;
    validate_snapshot_against_journal(&snapshot, &journal_state)?;
    if required_through_change_id > snapshot.high_watermark_change_id {
        bail!("analytics source snapshot does not cover the requested watermark");
    }
    let expired_pins = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM analytics_source_retention_pins \
         WHERE build_id = ?1 AND state = 'active' \
           AND julianday(expires_at) <= julianday('now'))",
    )
    .bind(build_id)
    .fetch_one(pool)
    .await?;
    if expired_pins != 0 {
        bail!("analytics source snapshot has an expired retention pin");
    }
    let covering_pins = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM analytics_source_retention_pins \
         WHERE build_id = ?1 AND state = 'active' \
           AND julianday(expires_at) > julianday('now') \
           AND watermark_change_id >= ?2)",
    )
    .bind(build_id)
    .bind(snapshot.high_watermark_change_id)
    .fetch_one(pool)
    .await?;
    if covering_pins == 0 {
        bail!("analytics source snapshot has no active retention pin covering its watermark");
    }
    Ok(snapshot)
}

fn validate_snapshot_against_journal(
    snapshot: &SourceSnapshot,
    journal_state: &SourceJournalStateRow,
) -> Result<()> {
    if snapshot.generation != journal_state.generation {
        bail!("analytics source snapshot generation is stale");
    }
    if journal_state.retained_from_change_id > snapshot.high_watermark_change_id {
        bail!("analytics source snapshot watermark is below the retained journal floor");
    }
    if journal_state.invalidation_pending != 0 {
        bail!("analytics source journal generation requires a rebuild");
    }
    if journal_state.event_count > journal_state.capacity {
        bail!("analytics source journal capacity is exceeded; target is unavailable");
    }
    Ok(())
}

#[derive(Debug, FromRow)]
struct SourceSnapshotRow {
    build_id: String,
    source_instance_id: String,
    generation: i64,
    high_watermark_change_id: i64,
    coverage_revision: i64,
    retained_from_change_id: i64,
    fact_semantics_version: i64,
    stats_semantics_version: i64,
    manifest_digest: Option<String>,
}

impl TryFrom<SourceSnapshotRow> for SourceSnapshot {
    type Error = anyhow::Error;

    fn try_from(row: SourceSnapshotRow) -> Result<Self> {
        let snapshot = Self {
            build_id: row.build_id,
            source_instance_id: row.source_instance_id,
            generation: row.generation,
            high_watermark_change_id: row.high_watermark_change_id,
            coverage_revision: row.coverage_revision,
            retained_from_change_id: row.retained_from_change_id,
            fact_semantics_version: row.fact_semantics_version,
            stats_semantics_version: row.stats_semantics_version,
            manifest_digest: row.manifest_digest,
        };
        validate_snapshot(&snapshot)?;
        Ok(snapshot)
    }
}

fn validate_snapshot(snapshot: &SourceSnapshot) -> Result<()> {
    if snapshot.build_id.trim().is_empty() || snapshot.build_id.chars().count() > BUILD_ID_MAX_CHARS
    {
        bail!("source snapshot build id is invalid");
    }
    if snapshot.source_instance_id.trim().is_empty() {
        bail!("source snapshot source instance id must not be empty");
    }
    if snapshot.generation < 0
        || snapshot.high_watermark_change_id < 0
        || snapshot.coverage_revision < 0
        || snapshot.retained_from_change_id < 0
    {
        bail!("source snapshot watermarks must not be negative");
    }
    if let Some(digest) = &snapshot.manifest_digest
        && (digest.trim().is_empty() || digest.chars().count() > MANIFEST_DIGEST_MAX_CHARS)
    {
        bail!("source snapshot manifest digest is invalid");
    }
    Ok(())
}

fn validate_pin(pin: &SourceRetentionPin) -> Result<()> {
    for (value, label) in [
        (&pin.pin_id, "pin id"),
        (&pin.build_id, "build id"),
        (&pin.source_kind, "source kind"),
        (&pin.source_identity, "source identity"),
    ] {
        if value.trim().is_empty() {
            bail!("source retention {label} must not be empty");
        }
    }
    if pin.watermark_change_id < 0 {
        bail!("source retention pin watermark must not be negative");
    }
    DateTime::parse_from_rfc3339(&pin.expires_at)
        .context("source retention pin expiry must be RFC3339")?;
    Ok(())
}
