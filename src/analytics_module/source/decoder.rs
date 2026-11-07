use super::{
    LEGACY_SOURCE_INSTANCE_ID, SourceEnvelope, SourceIdentity, SourceOperation, canonical_json,
    digest_json,
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use sqlx::FromRow;

/// The row shape accepted from the pre-envelope journal.  Keeping decoding explicit prevents a
/// legacy event from being interpreted as a current event merely because its JSON happens to
/// contain fields introduced by a newer writer.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct LegacyAnalyticsChange {
    pub(crate) change_id: i64,
    pub(crate) event_version: i64,
    pub(crate) fact_date_semantics_version: i64,
    pub(crate) event_kind: String,
    pub(crate) entity_kind: String,
    pub(crate) source: String,
    pub(crate) source_row_id: Option<i64>,
    pub(crate) entity_key: String,
    pub(crate) generation: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) occurred_at: String,
    pub(crate) fact_json: String,
}

pub(crate) fn decode_legacy_analytics_change(
    row: &LegacyAnalyticsChange,
) -> Result<SourceEnvelope> {
    if row.event_version != 1 {
        bail!(
            "unsupported legacy analytics event version {}",
            row.event_version
        );
    }
    if !matches!(row.event_kind.as_str(), "upsert" | "tombstone" | "coverage") {
        bail!("unsupported legacy analytics event kind {}", row.event_kind);
    }
    let payload = serde_json::from_str::<Value>(&row.fact_json)
        .context("decode legacy analytics fact JSON")?;
    if !payload.is_object() {
        bail!("legacy analytics fact must be a JSON object");
    }
    let source_instance_id = LEGACY_SOURCE_INSTANCE_ID.to_string();
    let operation = match row.event_kind.as_str() {
        "upsert" => SourceOperation::Upsert,
        "tombstone" => SourceOperation::LogicalRevoke,
        "coverage" => SourceOperation::Coverage,
        _ => unreachable!(),
    };
    let identity = SourceIdentity::new(
        source_instance_id.clone(),
        row.entity_kind.clone(),
        row.entity_key.clone(),
    )?;
    Ok(SourceEnvelope {
        envelope_version: 1,
        source_instance_id,
        event_id: format!("legacy:{}", row.change_id),
        generation: row.generation,
        change_id: row.change_id,
        coverage_revision: row.coverage_revision,
        fact_semantics_version: row.event_version,
        stats_semantics_version: row.fact_date_semantics_version,
        identity,
        operation,
        occurred_at: row.occurred_at.clone(),
        payload: canonical_json(&payload),
        payload_digest: digest_json(&payload)?,
    })
}

pub(crate) fn decode_legacy_event(row: &LegacyAnalyticsChange) -> Result<SourceEnvelope> {
    decode_legacy_analytics_change(row)
}

/// Decode the current source envelope and route the stats semantics to the strict typed reader.
///
/// `None` is the explicit legacy-stats result.  A typed entity carrying legacy stats is rejected
/// instead of being silently downgraded to a less capable payload.
pub(crate) fn decode_source_envelope(
    journal_event_version: i64,
    envelope: &SourceEnvelope,
) -> Result<Option<super::typed::TypedSourceKind>> {
    ensure!(
        journal_event_version == super::SOURCE_JOURNAL_EVENT_VERSION,
        "unsupported source journal event version {journal_event_version}"
    );
    ensure!(
        envelope.envelope_version == super::SOURCE_ENVELOPE_VERSION,
        "unsupported source envelope version {}",
        envelope.envelope_version
    );
    ensure!(
        envelope.fact_semantics_version == super::SOURCE_FACT_SEMANTICS_VERSION,
        "unsupported source fact semantics version {}",
        envelope.fact_semantics_version
    );
    match envelope.stats_semantics_version {
        super::typed::SOURCE_TYPED_STATS_SEMANTICS_VERSION => {
            super::typed::decode_typed_source_envelope(journal_event_version, envelope).map(Some)
        }
        super::SOURCE_STATS_SEMANTICS_VERSION => Ok(None),
        other => bail!("unsupported source stats semantics version {other}"),
    }
}
