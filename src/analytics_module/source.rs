//! SQLite-owned source protocol for the analytics module.
//!
//! The source journal is the durable boundary between business facts and analytics-derived
//! state.  This module keeps event identity, operation semantics, and the source snapshot APIs
//! independent from any particular PostgreSQL consumer.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqliteConnection};

#[path = "source/decoder.rs"]
pub(crate) mod decoder;
#[path = "source/snapshot.rs"]
pub(crate) mod snapshot;

#[allow(unused_imports)]
pub(crate) use decoder::{
    LegacyAnalyticsChange, decode_legacy_analytics_change, decode_legacy_event,
};
#[allow(unused_imports)]
pub(crate) use snapshot::{
    SourceRetentionPin, SourceSnapshot, assert_source_snapshot_usable, create_source_snapshot,
    create_source_snapshot_tx, ensure_source_snapshot_schema, load_source_snapshot,
    release_source_retention_pin, release_source_retention_pin_tx, renew_source_retention_pin,
    renew_source_retention_pin_tx,
};

pub(crate) const SOURCE_ENVELOPE_VERSION: i64 = 2;
pub(crate) const SOURCE_JOURNAL_EVENT_VERSION: i64 = 1;
pub(crate) const SOURCE_FACT_SEMANTICS_VERSION: i64 = 1;
pub(crate) const SOURCE_STATS_SEMANTICS_VERSION: i64 = 2;
pub(crate) const LEGACY_SOURCE_INSTANCE_ID: &str = "legacy-sqlite";
const SOURCE_TEXT_MAX_CHARS: usize = 256;
const SOURCE_ENTITY_KEY_MAX_BYTES: usize = 512;
const SOURCE_PAYLOAD_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceOperation {
    Upsert,
    Correction,
    LogicalRevoke,
    ArchiveRelocation,
    RetentionRetirement,
    Coverage,
}

impl SourceOperation {
    pub(crate) fn event_kind(self) -> &'static str {
        match self {
            Self::Upsert | Self::Correction => "upsert",
            Self::LogicalRevoke => "tombstone",
            Self::ArchiveRelocation | Self::RetentionRetirement | Self::Coverage => "coverage",
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::Correction => "correction",
            Self::LogicalRevoke => "logical_revoke",
            Self::ArchiveRelocation => "archive_relocation",
            Self::RetentionRetirement => "retention_retirement",
            Self::Coverage => "coverage",
        }
    }

    pub(crate) fn preserves_contribution(self) -> bool {
        matches!(self, Self::ArchiveRelocation | Self::RetentionRetirement)
    }

    pub(crate) fn is_correction(self) -> bool {
        matches!(self, Self::Correction)
    }

    pub(crate) fn is_deletion(self) -> bool {
        matches!(self, Self::LogicalRevoke)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceIdentity {
    pub(crate) source_instance_id: String,
    pub(crate) entity_kind: String,
    pub(crate) entity_key: String,
}

impl SourceIdentity {
    pub(crate) fn new(
        source_instance_id: impl Into<String>,
        entity_kind: impl Into<String>,
        entity_key: impl Into<String>,
    ) -> Result<Self> {
        let identity = Self {
            source_instance_id: source_instance_id.into(),
            entity_kind: entity_kind.into(),
            entity_key: entity_key.into(),
        };
        identity.validate()?;
        Ok(identity)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        validate_text(&self.source_instance_id, "source instance id")?;
        validate_text(&self.entity_kind, "entity kind")?;
        let entity_key = self.entity_key.trim();
        if entity_key.is_empty() {
            bail!("source entity key must not be empty");
        }
        if entity_key.len() > SOURCE_ENTITY_KEY_MAX_BYTES {
            bail!("source entity key exceeds the bounded size");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceEnvelopeDraft {
    pub(crate) source_instance_id: String,
    pub(crate) event_id: String,
    pub(crate) entity_kind: String,
    pub(crate) entity_key: String,
    pub(crate) operation: SourceOperation,
    pub(crate) occurred_at: String,
    pub(crate) payload: Value,
}

impl SourceEnvelopeDraft {
    pub(crate) fn identity(&self) -> Result<SourceIdentity> {
        SourceIdentity::new(
            self.source_instance_id.clone(),
            self.entity_kind.clone(),
            self.entity_key.clone(),
        )
    }

    pub(crate) fn validate(&self) -> Result<Value> {
        self.identity()?.validate()?;
        validate_text(&self.event_id, "event id")?;
        validate_text(&self.occurred_at, "occurred_at")?;
        if !self.payload.is_object() {
            bail!("source envelope payload must be a JSON object");
        }
        let payload = canonical_json(&self.payload);
        let encoded = serde_json::to_vec(&payload).context("encode source envelope payload")?;
        if encoded.len() > SOURCE_PAYLOAD_MAX_BYTES {
            bail!("source envelope payload exceeds the bounded size");
        }
        Ok(payload)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceEnvelope {
    pub(crate) envelope_version: i64,
    pub(crate) source_instance_id: String,
    pub(crate) event_id: String,
    pub(crate) generation: i64,
    pub(crate) change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) fact_semantics_version: i64,
    pub(crate) stats_semantics_version: i64,
    pub(crate) identity: SourceIdentity,
    pub(crate) operation: SourceOperation,
    pub(crate) occurred_at: String,
    pub(crate) payload: Value,
    pub(crate) payload_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceAppendReceipt {
    pub(crate) change_id: i64,
    pub(crate) generation: i64,
    pub(crate) coverage_revision: i64,
}

#[derive(Debug, FromRow)]
struct SourceStateRow {
    generation: i64,
    coverage_revision: i64,
    retained_from_change_id: i64,
    capacity: i64,
}

#[derive(Debug, FromRow)]
struct ExistingSourceEventRow {
    change_id: i64,
    event_id: String,
    generation: i64,
    coverage_revision: i64,
    source_instance_id: String,
    payload_digest: String,
    fact_semantics_version: i64,
    stats_semantics_version: i64,
    operation_kind: String,
    entity_kind: String,
    entity_key: String,
    occurred_at: String,
}

pub(crate) async fn append_source_envelope_tx(
    connection: &mut SqliteConnection,
    draft: &SourceEnvelopeDraft,
) -> Result<(SourceEnvelope, SourceAppendReceipt)> {
    let payload = draft.validate()?;
    let identity = draft.identity()?;
    let payload_digest = digest_json(&payload)?;

    let existing = sqlx::query_as::<_, ExistingSourceEventRow>(
        "SELECT change_id, event_id, generation, coverage_revision, source_instance_id, payload_digest, \
         fact_semantics_version, stats_semantics_version, operation_kind, entity_kind, entity_key, occurred_at \
         FROM analytics_change_events WHERE event_id = ?1 LIMIT 1",
    )
    .bind(&draft.event_id)
    .fetch_optional(&mut *connection)
    .await
    .context("load existing source envelope identity")?;
    if let Some(existing) = existing {
        if existing.source_instance_id != draft.source_instance_id
            || existing.payload_digest != payload_digest
            || existing.operation_kind != draft.operation.as_str()
            || existing.entity_kind != draft.entity_kind
            || existing.entity_key != draft.entity_key
            || existing.occurred_at != draft.occurred_at
        {
            bail!("source event identity conflicts with an existing payload");
        }
        let envelope = envelope_from_row(existing, payload)?;
        let receipt = SourceAppendReceipt {
            change_id: envelope.change_id,
            generation: envelope.generation,
            coverage_revision: envelope.coverage_revision,
        };
        return Ok((envelope, receipt));
    }

    let state = sqlx::query_as::<_, SourceStateRow>(
        "SELECT generation, coverage_revision, retained_from_change_id, capacity \
         FROM analytics_change_state WHERE id = 1",
    )
    .fetch_one(&mut *connection)
    .await
    .context("load analytics source state")?;
    let latest_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM analytics_change_events")
        .fetch_one(&mut *connection)
        .await?;
    if latest_count >= state.capacity {
        let oldest = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MIN(change_id) FROM analytics_change_events",
        )
        .fetch_one(&mut *connection)
        .await?
        .unwrap_or(state.retained_from_change_id);
        let pinned = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM analytics_source_retention_pins \
             WHERE state = 'active' AND julianday(expires_at) > julianday('now') \
               AND watermark_change_id >= ?1)",
        )
        .bind(oldest)
        .fetch_one(&mut *connection)
        .await
        .unwrap_or(1);
        if pinned != 0 {
            bail!("analytics source journal capacity is pinned at its retained boundary");
        }
        bail!("analytics source journal capacity exhausted; rebuild or advance retention");
    }

    let coverage_revision = state
        .coverage_revision
        .checked_add(1)
        .context("analytics source coverage revision overflow")?;
    let fact_json = serde_json::to_string(&payload).context("encode source envelope payload")?;
    let operation_kind = draft.operation.as_str();
    let result = sqlx::query(
        "INSERT INTO analytics_change_events (event_version, fact_date_semantics_version, \
         source_instance_id, event_id, operation_kind, fact_semantics_version, \
         stats_semantics_version, payload_digest, event_kind, entity_kind, source, source_row_id, \
         entity_key, generation, coverage_revision, occurred_at, fact_json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?13, ?14, ?15, ?16)",
    )
    .bind(SOURCE_JOURNAL_EVENT_VERSION)
    .bind(SOURCE_STATS_SEMANTICS_VERSION)
    .bind(&draft.source_instance_id)
    .bind(&draft.event_id)
    .bind(operation_kind)
    .bind(SOURCE_FACT_SEMANTICS_VERSION)
    .bind(SOURCE_STATS_SEMANTICS_VERSION)
    .bind(&payload_digest)
    .bind(draft.operation.event_kind())
    .bind(&draft.entity_kind)
    .bind(&draft.source_instance_id)
    .bind(&draft.entity_key)
    .bind(state.generation)
    .bind(coverage_revision)
    .bind(&draft.occurred_at)
    .bind(&fact_json)
    .execute(&mut *connection)
    .await
    .context("append analytics source envelope")?;
    let change_id = result.last_insert_rowid();
    sqlx::query(
        "UPDATE analytics_change_state SET coverage_revision = ?1, updated_at = datetime('now') WHERE id = 1",
    )
    .bind(coverage_revision)
    .execute(&mut *connection)
    .await
    .context("advance analytics source coverage revision")?;

    let envelope = SourceEnvelope {
        envelope_version: SOURCE_ENVELOPE_VERSION,
        source_instance_id: draft.source_instance_id.clone(),
        event_id: draft.event_id.clone(),
        generation: state.generation,
        change_id,
        coverage_revision,
        fact_semantics_version: SOURCE_FACT_SEMANTICS_VERSION,
        stats_semantics_version: SOURCE_STATS_SEMANTICS_VERSION,
        identity,
        operation: draft.operation,
        occurred_at: draft.occurred_at.clone(),
        payload,
        payload_digest,
    };
    let receipt = SourceAppendReceipt {
        change_id,
        generation: state.generation,
        coverage_revision,
    };
    Ok((envelope, receipt))
}

pub(crate) fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        Value::Object(object) => {
            let mut sorted = Map::new();
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                sorted.insert(key.clone(), canonical_json(value));
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

pub(crate) fn digest_json(value: &Value) -> Result<String> {
    let canonical = canonical_json(value);
    let bytes = serde_json::to_vec(&canonical).context("encode canonical source payload")?;
    let mut digest = Sha256::new();
    digest.update(bytes);
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_text(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        bail!("source {label} must not be empty");
    }
    if value.chars().count() > SOURCE_TEXT_MAX_CHARS {
        bail!("source {label} exceeds the bounded size");
    }
    Ok(())
}

fn envelope_from_row(row: ExistingSourceEventRow, payload: Value) -> Result<SourceEnvelope> {
    let operation = match row.operation_kind.as_str() {
        "upsert" => SourceOperation::Upsert,
        "correction" => SourceOperation::Correction,
        "logical_revoke" => SourceOperation::LogicalRevoke,
        "archive_relocation" => SourceOperation::ArchiveRelocation,
        "retention_retirement" => SourceOperation::RetentionRetirement,
        "coverage" | "archive_coverage" => SourceOperation::Coverage,
        other => bail!("unsupported persisted source operation: {other}"),
    };
    let identity = SourceIdentity::new(
        row.source_instance_id.clone(),
        row.entity_kind,
        row.entity_key,
    )?;
    Ok(SourceEnvelope {
        envelope_version: SOURCE_ENVELOPE_VERSION,
        source_instance_id: row.source_instance_id,
        event_id: row.event_id,
        generation: row.generation,
        change_id: row.change_id,
        coverage_revision: row.coverage_revision,
        fact_semantics_version: row.fact_semantics_version,
        stats_semantics_version: row.stats_semantics_version,
        identity,
        operation,
        occurred_at: row.occurred_at,
        payload,
        payload_digest: row.payload_digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Pool, Sqlite, sqlite::SqlitePoolOptions};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect source protocol test database");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("ensure source protocol schema");
        ensure_source_snapshot_schema(&pool)
            .await
            .expect("ensure source snapshot schema");
        pool
    }

    #[test]
    fn deletion_operations_have_distinct_projection_semantics() {
        assert!(SourceOperation::LogicalRevoke.is_deletion());
        assert!(!SourceOperation::LogicalRevoke.preserves_contribution());
        assert!(SourceOperation::Correction.is_correction());
        assert_eq!(SourceOperation::Correction.event_kind(), "upsert");
        assert!(SourceOperation::ArchiveRelocation.preserves_contribution());
        assert!(SourceOperation::RetentionRetirement.preserves_contribution());
        assert_eq!(SourceOperation::LogicalRevoke.event_kind(), "tombstone");
        assert_eq!(SourceOperation::ArchiveRelocation.event_kind(), "coverage");
    }

    #[tokio::test]
    async fn append_is_idempotent_and_keeps_envelope_version_separate() {
        let pool = test_pool().await;
        let draft = SourceEnvelopeDraft {
            source_instance_id: "sqlite-instance-a".to_string(),
            event_id: "event-0001".to_string(),
            entity_kind: "invocation".to_string(),
            entity_key: "invocation:proxy:42".to_string(),
            operation: SourceOperation::Upsert,
            occurred_at: "2026-11-05T02:00:00Z".to_string(),
            payload: serde_json::json!({"z": 1, "a": {"second": 2, "first": 1}}),
        };
        let mut tx = pool.begin().await.expect("begin source append");
        let (first, first_receipt) = append_source_envelope_tx(tx.as_mut(), &draft)
            .await
            .expect("append first source envelope");
        tx.commit().await.expect("commit first source append");

        assert_eq!(first.envelope_version, SOURCE_ENVELOPE_VERSION);
        assert_eq!(first_receipt.change_id, first.change_id);
        let journal_version: i64 = sqlx::query_scalar(
            "SELECT event_version FROM analytics_change_events WHERE event_id = ?1",
        )
        .bind(&draft.event_id)
        .fetch_one(&pool)
        .await
        .expect("load journal event version");
        assert_eq!(journal_version, SOURCE_JOURNAL_EVENT_VERSION);

        let mut tx = pool.begin().await.expect("begin repeated source append");
        let (second, second_receipt) = append_source_envelope_tx(tx.as_mut(), &draft)
            .await
            .expect("append repeated source envelope");
        tx.commit().await.expect("commit repeated source append");
        assert_eq!(second.change_id, first.change_id);
        assert_eq!(second_receipt, first_receipt);
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM analytics_change_events WHERE event_id = ?1")
                .bind(&draft.event_id)
                .fetch_one(&pool)
                .await
                .expect("count repeated source event");
        assert_eq!(count, 1);

        let conflicting = SourceEnvelopeDraft {
            entity_key: "invocation:proxy:43".to_string(),
            ..draft.clone()
        };
        let mut tx = pool.begin().await.expect("begin conflicting source append");
        let error = append_source_envelope_tx(tx.as_mut(), &conflicting)
            .await
            .expect_err("reject reused event identity");
        assert!(error.to_string().contains("identity conflicts"));
        tx.rollback()
            .await
            .expect("rollback conflicting source append");
    }

    #[tokio::test]
    async fn append_rolls_back_without_leaving_an_event_or_revision() {
        let pool = test_pool().await;
        let draft = SourceEnvelopeDraft {
            source_instance_id: "sqlite-instance-a".to_string(),
            event_id: "event-rollback".to_string(),
            entity_kind: "invocation".to_string(),
            entity_key: "invocation:proxy:rollback".to_string(),
            operation: SourceOperation::Correction,
            occurred_at: "2026-11-05T02:00:00Z".to_string(),
            payload: serde_json::json!({"corrected": true}),
        };
        let mut tx = pool.begin().await.expect("begin rollback source append");
        append_source_envelope_tx(tx.as_mut(), &draft)
            .await
            .expect("append source event before rollback");
        tx.rollback().await.expect("rollback source append");

        let event_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM analytics_change_events WHERE event_id = ?1")
                .bind(&draft.event_id)
                .fetch_one(&pool)
                .await
                .expect("count rolled-back source event");
        let coverage_revision: i64 =
            sqlx::query_scalar("SELECT coverage_revision FROM analytics_change_state WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load rolled-back coverage revision");
        assert_eq!(event_count, 0);
        assert_eq!(coverage_revision, 0);
    }

    #[tokio::test]
    async fn snapshot_pin_covers_a_build_until_explicit_release() {
        let pool = test_pool().await;
        let snapshot = SourceSnapshot {
            build_id: "build-0001".to_string(),
            source_instance_id: "sqlite-instance-a".to_string(),
            generation: 3,
            high_watermark_change_id: 42,
            coverage_revision: 17,
            retained_from_change_id: 10,
            fact_semantics_version: SOURCE_FACT_SEMANTICS_VERSION,
            stats_semantics_version: SOURCE_STATS_SEMANTICS_VERSION,
            manifest_digest: Some("sha256:manifest".to_string()),
        };
        let pin = SourceRetentionPin {
            pin_id: "pin-0001".to_string(),
            build_id: snapshot.build_id.clone(),
            source_kind: "archive_manifest".to_string(),
            source_identity: "archive:codex_invocations:42".to_string(),
            watermark_change_id: 42,
            expires_at: "2099-01-01T00:00:00Z".to_string(),
        };
        let mut tx = pool.begin().await.expect("begin snapshot transaction");
        create_source_snapshot_tx(tx.as_mut(), &snapshot)
            .await
            .expect("create source snapshot");
        tx.commit().await.expect("commit snapshot transaction");

        let error = assert_source_snapshot_usable(
            &pool,
            &snapshot.build_id,
            &snapshot.source_instance_id,
            snapshot.generation,
            40,
        )
        .await
        .expect_err("reject a snapshot without a retention pin");
        assert!(error.to_string().contains("retention pin"));

        let mut tx = pool.begin().await.expect("begin pin transaction");
        renew_source_retention_pin_tx(tx.as_mut(), &pin)
            .await
            .expect("renew source retention pin");
        tx.commit().await.expect("commit pin transaction");

        let loaded = assert_source_snapshot_usable(
            &pool,
            &snapshot.build_id,
            &snapshot.source_instance_id,
            snapshot.generation,
            40,
        )
        .await
        .expect("validate pinned snapshot");
        assert_eq!(loaded, snapshot);
        assert_eq!(
            load_source_snapshot(&pool, &snapshot.build_id)
                .await
                .expect("load source snapshot"),
            Some(snapshot.clone())
        );

        sqlx::query(
            "UPDATE analytics_source_retention_pins SET watermark_change_id = 41 WHERE pin_id = ?1",
        )
        .bind(&pin.pin_id)
        .execute(&pool)
        .await
        .expect("lower retention pin watermark");
        let error = assert_source_snapshot_usable(
            &pool,
            &snapshot.build_id,
            &snapshot.source_instance_id,
            snapshot.generation,
            40,
        )
        .await
        .expect_err("reject a pin below the snapshot watermark");
        assert!(error.to_string().contains("covering its watermark"));

        sqlx::query(
            "UPDATE analytics_source_retention_pins SET watermark_change_id = 42, \
             expires_at = '2000-01-01T00:00:00Z' WHERE pin_id = ?1",
        )
        .bind(&pin.pin_id)
        .execute(&pool)
        .await
        .expect("expire retention pin");
        let error = assert_source_snapshot_usable(
            &pool,
            &snapshot.build_id,
            &snapshot.source_instance_id,
            snapshot.generation,
            40,
        )
        .await
        .expect_err("reject an expired retention pin");
        assert!(error.to_string().contains("expired retention pin"));

        let mut tx = pool.begin().await.expect("begin pin renewal");
        renew_source_retention_pin_tx(tx.as_mut(), &pin)
            .await
            .expect("renew expired retention pin");
        tx.commit().await.expect("commit pin renewal");
        assert_source_snapshot_usable(
            &pool,
            &snapshot.build_id,
            &snapshot.source_instance_id,
            snapshot.generation,
            40,
        )
        .await
        .expect("accept a renewed covering retention pin");

        let mut tx = pool.begin().await.expect("begin pin release");
        assert!(
            release_source_retention_pin_tx(tx.as_mut(), &pin.pin_id)
                .await
                .expect("release source retention pin")
        );
        tx.commit().await.expect("commit pin release");

        let error = assert_source_snapshot_usable(
            &pool,
            &snapshot.build_id,
            &snapshot.source_instance_id,
            snapshot.generation,
            40,
        )
        .await
        .expect_err("reject a released retention pin");
        assert!(error.to_string().contains("retention pin"));
    }

    #[test]
    fn legacy_decoder_maps_old_tombstone_to_logical_revoke() {
        let row = LegacyAnalyticsChange {
            change_id: 9,
            event_version: 1,
            fact_date_semantics_version: 1,
            event_kind: "tombstone".to_string(),
            entity_kind: "invocation".to_string(),
            source: "proxy".to_string(),
            source_row_id: Some(42),
            entity_key: "invocation:proxy:42".to_string(),
            generation: 2,
            coverage_revision: 8,
            occurred_at: "2026-11-05T02:00:00Z".to_string(),
            fact_json: "{\"deleted\":true}".to_string(),
        };
        let decoded = decode_legacy_analytics_change(&row).expect("decode legacy event");
        assert_eq!(decoded.envelope_version, 1);
        assert_eq!(decoded.operation, SourceOperation::LogicalRevoke);
        assert_eq!(decoded.event_id, "legacy:9");
        assert_eq!(
            decoded.identity.source_instance_id,
            LEGACY_SOURCE_INSTANCE_ID
        );
    }
}
