use super::{SourceEnvelope, SourceIdentity, SourceOperation, canonical_json, digest_json};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::SqliteConnection;

pub(crate) const SOURCE_TYPED_STATS_SEMANTICS_VERSION: i64 = 3;
pub(crate) const SOURCE_INVOCATION_FACT_VERSION: i64 = 2;
pub(crate) const SOURCE_POOL_ATTEMPT_FACT_VERSION: i64 = 2;
pub(crate) const SOURCE_NETWORK_MINUTE_FACT_VERSION: i64 = 1;
pub(crate) const SOURCE_PROXY_WEIGHT_HOURLY_FACT_VERSION: i64 = 1;
pub(crate) const SOURCE_FORWARD_PROXY_ATTEMPT_FACT_VERSION: i64 = 1;
pub(crate) const SOURCE_FILTER_SNAPSHOT_VERSION: i64 = 1;

const INVOCATION_FIELDS: &[&str] = &[
    "factVersion",
    "source",
    "sourceRowId",
    "invokeId",
    "occurredAt",
    "occurredAtEpochMs",
    "statsDate",
    "model",
    "requestModel",
    "responseModel",
    "reasoningEffort",
    "promptCacheKeyDigest",
    "upstreamAccountId",
    "upstreamAccountName",
    "status",
    "successLike",
    "terminal",
    "failureKind",
    "failureClass",
    "isActionable",
    "inputTokens",
    "outputTokens",
    "cacheInputTokens",
    "reasoningTokens",
    "totalTokens",
    "cost",
    "costInput",
    "costCacheWrite",
    "costCacheRead",
    "costOutput",
    "costReasoning",
    "tTotalMs",
    "tReqReadMs",
    "tReqParseMs",
    "tUpstreamConnectMs",
    "tUpstreamTtfbMs",
    "tUpstreamStreamMs",
    "firstTokenMs",
    "tRespParseMs",
    "filterSnapshot",
    "deleted",
];

const FILTER_SNAPSHOT_FIELDS: &[&str] = &[
    "version",
    "endpoint",
    "upstreamScope",
    "transport",
    "serviceTier",
    "serviceTierSnake",
    "proxyDisplayName",
    "stickyKey",
    "requesterIp",
    "promptCacheKey",
    "invokeId",
    "model",
    "failureKind",
    "errorMessage",
    "downstreamErrorMessage",
    "requestModel",
    "responseModel",
    "status",
    "failureClass",
    "reasoning",
    "upstreamAccountId",
    "upstreamAccountName",
    "reasoningEffort",
];

const POOL_ATTEMPT_FIELDS: &[&str] = &[
    "factVersion",
    "source",
    "sourceRowId",
    "attemptPublicId",
    "invokeId",
    "occurredAt",
    "endpoint",
    "requestModel",
    "upstreamRequestModel",
    "upstreamAccountId",
    "attemptIndex",
    "distinctAccountIndex",
    "sameAccountRetryIndex",
    "status",
    "phase",
    "httpStatus",
    "downstreamHttpStatus",
    "failureKind",
    "connectLatencyMs",
    "firstByteLatencyMs",
    "streamLatencyMs",
    "proxyId",
    "proxyBindingKey",
    "finishedAt",
    "eligible",
    "deleted",
];

const NETWORK_MINUTE_FIELDS: &[&str] = &[
    "factVersion",
    "bucketStartEpochSecond",
    "source",
    "upstreamBaseUrlHost",
    "upstreamAccountId",
    "uploadBytes",
    "downloadBytes",
    "deleted",
];

const PROXY_WEIGHT_HOURLY_FIELDS: &[&str] = &[
    "factVersion",
    "proxyId",
    "bucketStartEpochSecond",
    "sampleCount",
    "minWeight",
    "maxWeight",
    "avgWeight",
    "lastWeight",
    "lastSampleEpochUs",
    "deleted",
];

const FORWARD_PROXY_ATTEMPT_FIELDS: &[&str] = &[
    "factVersion",
    "source",
    "sourceRowId",
    "proxyKey",
    "occurredAt",
    "isSuccess",
    "latencyMs",
    "failureKind",
    "isProbe",
    "deleted",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TypedSourceKind {
    Invocation,
    PoolAttempt,
    NetworkMinute,
    ProxyWeightHourly,
    ForwardProxyAttempt,
}

impl TypedSourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Invocation => "invocation",
            Self::PoolAttempt => "pool_attempt",
            Self::NetworkMinute => "network_minute",
            Self::ProxyWeightHourly => "proxy_weight_hourly",
            Self::ForwardProxyAttempt => "forward_proxy_attempt",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "invocation" => Ok(Self::Invocation),
            "pool_attempt" => Ok(Self::PoolAttempt),
            "network_minute" => Ok(Self::NetworkMinute),
            "proxy_weight_hourly" => Ok(Self::ProxyWeightHourly),
            "forward_proxy_attempt" => Ok(Self::ForwardProxyAttempt),
            other => bail!("unknown typed source entity kind: {other}"),
        }
    }

    fn fact_version(self) -> i64 {
        match self {
            Self::Invocation => SOURCE_INVOCATION_FACT_VERSION,
            Self::PoolAttempt => SOURCE_POOL_ATTEMPT_FACT_VERSION,
            Self::NetworkMinute => SOURCE_NETWORK_MINUTE_FACT_VERSION,
            Self::ProxyWeightHourly => SOURCE_PROXY_WEIGHT_HOURLY_FACT_VERSION,
            Self::ForwardProxyAttempt => SOURCE_FORWARD_PROXY_ATTEMPT_FACT_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TypedSourceEnvelopeDraft {
    pub(crate) source_instance_id: String,
    pub(crate) event_id: String,
    pub(crate) entity_kind: String,
    pub(crate) entity_key: String,
    pub(crate) operation: SourceOperation,
    pub(crate) occurred_at: String,
    pub(crate) payload: Value,
}

impl TypedSourceEnvelopeDraft {
    pub(crate) fn new(
        source_instance_id: impl Into<String>,
        event_id: impl Into<String>,
        kind: TypedSourceKind,
        entity_key: impl Into<String>,
        operation: SourceOperation,
        occurred_at: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            source_instance_id: source_instance_id.into(),
            event_id: event_id.into(),
            entity_kind: kind.as_str().to_string(),
            entity_key: entity_key.into(),
            operation,
            occurred_at: occurred_at.into(),
            payload,
        }
    }

    pub(crate) fn kind(&self) -> Result<TypedSourceKind> {
        TypedSourceKind::parse(&self.entity_kind)
    }

    pub(crate) fn validate(&self) -> Result<Value> {
        SourceIdentity::new(
            self.source_instance_id.clone(),
            self.entity_kind.clone(),
            self.entity_key.clone(),
        )?;
        super::validate_text(&self.event_id, "event id")?;
        super::validate_text(&self.occurred_at, "occurred_at")?;
        ensure!(
            matches!(
                self.operation,
                SourceOperation::Upsert
                    | SourceOperation::Correction
                    | SourceOperation::LogicalRevoke
                    | SourceOperation::ArchiveRelocation
                    | SourceOperation::RetentionRetirement
            ),
            "typed source operation is not a fact operation"
        );
        let kind = self.kind()?;
        let payload = canonical_json(&self.payload);
        validate_typed_source_payload(kind, &payload)?;
        validate_typed_entity_key(kind, &self.entity_key, &payload)?;
        if self.operation.is_deletion() {
            ensure!(
                payload.get("deleted").and_then(Value::as_bool) == Some(true),
                "typed tombstone payload must mark deleted=true"
            );
        } else {
            ensure!(
                payload.get("deleted").and_then(Value::as_bool) != Some(true),
                "non-tombstone typed payload cannot mark deleted=true"
            );
        }
        serde_json::to_vec(&payload).context("encode typed source payload")?;
        Ok(payload)
    }
}

pub(crate) fn typed_source_payload_encoded_size(payload: &Value) -> Result<usize> {
    serde_json::to_vec(&canonical_json(payload))
        .map(|encoded| encoded.len())
        .context("encode typed source payload for byte accounting")
}

pub(crate) fn validate_typed_source_payload(kind: TypedSourceKind, payload: &Value) -> Result<()> {
    let object = payload
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("typed source payload must be a JSON object"))?;
    match kind {
        TypedSourceKind::Invocation => validate_invocation(object),
        TypedSourceKind::PoolAttempt => validate_pool_attempt(object),
        TypedSourceKind::NetworkMinute => validate_network_minute(object),
        TypedSourceKind::ProxyWeightHourly => validate_proxy_weight_hourly(object),
        TypedSourceKind::ForwardProxyAttempt => validate_forward_proxy_attempt(object),
    }
}

pub(crate) fn decode_typed_source_envelope(
    journal_event_version: i64,
    envelope: &SourceEnvelope,
) -> Result<TypedSourceKind> {
    ensure!(
        journal_event_version == super::SOURCE_JOURNAL_EVENT_VERSION,
        "unsupported typed source journal event version {}",
        journal_event_version
    );
    ensure!(
        envelope.envelope_version == super::SOURCE_ENVELOPE_VERSION,
        "unsupported typed source envelope version {}",
        envelope.envelope_version
    );
    ensure!(
        envelope.fact_semantics_version == super::SOURCE_FACT_SEMANTICS_VERSION,
        "unsupported typed source fact semantics version {}",
        envelope.fact_semantics_version
    );
    ensure!(
        envelope.stats_semantics_version == SOURCE_TYPED_STATS_SEMANTICS_VERSION,
        "typed source requires stats semantics version {}",
        SOURCE_TYPED_STATS_SEMANTICS_VERSION
    );
    ensure!(
        matches!(
            envelope.operation,
            SourceOperation::Upsert
                | SourceOperation::Correction
                | SourceOperation::LogicalRevoke
                | SourceOperation::ArchiveRelocation
                | SourceOperation::RetentionRetirement
        ),
        "typed source envelope carries a non-fact operation"
    );
    let kind = TypedSourceKind::parse(&envelope.identity.entity_kind)?;
    validate_typed_source_payload(kind, &envelope.payload)?;
    validate_typed_entity_key(kind, &envelope.identity.entity_key, &envelope.payload)?;
    if envelope.operation.is_deletion() {
        ensure!(
            envelope.payload.get("deleted").and_then(Value::as_bool) == Some(true),
            "typed tombstone payload must mark deleted=true"
        );
    } else {
        ensure!(
            envelope.payload.get("deleted").and_then(Value::as_bool) != Some(true),
            "non-tombstone typed payload cannot mark deleted=true"
        );
    }
    ensure!(
        digest_json(&envelope.payload)? == envelope.payload_digest,
        "typed source payload digest does not match the canonical payload"
    );
    Ok(kind)
}

pub(crate) async fn assert_typed_entity_key_collision_free(
    connection: &mut SqliteConnection,
    draft: &TypedSourceEnvelopeDraft,
    payload: &Value,
) -> Result<()> {
    let kind = draft.kind()?;
    let Some(existing_fact_json) = sqlx::query_scalar::<_, String>(
        "SELECT fact_json FROM analytics_change_events \
         WHERE entity_kind = ?1 AND entity_key = ?2 \
         ORDER BY change_id DESC LIMIT 1",
    )
    .bind(&draft.entity_kind)
    .bind(&draft.entity_key)
    .fetch_optional(&mut *connection)
    .await
    .context("load typed source physical identity")?
    else {
        return Ok(());
    };
    let existing_payload = serde_json::from_str::<Value>(&existing_fact_json)
        .context("decode existing typed source key payload")?;
    if matches!(
        kind,
        TypedSourceKind::NetworkMinute | TypedSourceKind::ProxyWeightHourly
    ) {
        ensure!(
            physical_identity(kind, &existing_payload)? == physical_identity(kind, payload)?,
            "typed source entity key collision changes the physical identity"
        );
    }
    Ok(())
}

pub(crate) fn network_minute_entity_key(
    bucket_start_epoch_second: i64,
    source: &str,
    upstream_base_url_host: Option<&str>,
    upstream_account_id: Option<i64>,
) -> Result<String> {
    let tuple = json!([
        bucket_start_epoch_second,
        source,
        upstream_base_url_host,
        upstream_account_id
    ]);
    opaque_entity_key("network_minute", &tuple)
}

pub(crate) fn pool_attempt_entity_key(source_row_id: i64) -> Result<String> {
    ensure!(
        source_row_id > 0,
        "pool attempt source row id must be positive"
    );
    Ok(format!("attempt:{source_row_id}"))
}

pub(crate) fn proxy_weight_hourly_entity_key(
    proxy_id: &str,
    bucket_start_epoch_second: i64,
) -> Result<String> {
    ensure!(
        !proxy_id.is_empty(),
        "proxy weight proxy id must not be empty"
    );
    let tuple = json!([proxy_id, bucket_start_epoch_second]);
    opaque_entity_key("proxy_weight_hourly", &tuple)
}

pub(crate) fn forward_proxy_attempt_entity_key(source_row_id: i64) -> Result<String> {
    ensure!(
        source_row_id > 0,
        "forward proxy attempt source row id must be positive"
    );
    Ok(format!("forward_proxy_attempt:{source_row_id}"))
}

fn validate_typed_entity_key(
    kind: TypedSourceKind,
    entity_key: &str,
    payload: &Value,
) -> Result<()> {
    match kind {
        TypedSourceKind::Invocation => {
            let source = required_nonempty_text_value(payload, "source")?;
            let source_row_id = required_i64_value(payload, "sourceRowId")?;
            ensure!(
                entity_key == format!("invocation:{source}:{source_row_id}"),
                "invocation typed entity key does not match source identity"
            );
        }
        TypedSourceKind::PoolAttempt => {
            let source_row_id = required_i64_value(payload, "sourceRowId")?;
            ensure!(
                entity_key == pool_attempt_entity_key(source_row_id)?,
                "pool attempt typed entity key does not match source identity"
            );
        }
        TypedSourceKind::NetworkMinute => {
            let bucket = required_i64_value(payload, "bucketStartEpochSecond")?;
            let source = required_nonempty_text_value(payload, "source")?;
            let host = nullable_text_value(payload, "upstreamBaseUrlHost")?;
            let account = nullable_i64_value(payload, "upstreamAccountId")?;
            ensure!(
                entity_key == network_minute_entity_key(bucket, source, host, account)?,
                "network minute typed entity key does not match physical partition"
            );
        }
        TypedSourceKind::ProxyWeightHourly => {
            let proxy_id = required_nonempty_text_value(payload, "proxyId")?;
            let bucket = required_i64_value(payload, "bucketStartEpochSecond")?;
            ensure!(
                entity_key == proxy_weight_hourly_entity_key(proxy_id, bucket)?,
                "proxy weight typed entity key does not match physical bucket"
            );
        }
        TypedSourceKind::ForwardProxyAttempt => {
            ensure!(
                required_nonempty_text_value(payload, "source")? == "forward_proxy_attempts",
                "forward proxy attempt source identity is not the forward_proxy_attempts table"
            );
            let source_row_id = required_i64_value(payload, "sourceRowId")?;
            ensure!(
                entity_key == forward_proxy_attempt_entity_key(source_row_id)?,
                "forward proxy attempt typed entity key does not match source identity"
            );
        }
    }
    Ok(())
}

fn validate_invocation(object: &Map<String, Value>) -> Result<()> {
    ensure_known_fields(object, INVOCATION_FIELDS, "invocation")?;
    ensure_fact_version(object, TypedSourceKind::Invocation)?;
    for field in [
        "source",
        "invokeId",
        "occurredAt",
        "statsDate",
        "model",
        "requestModel",
        "responseModel",
        "reasoningEffort",
        "promptCacheKeyDigest",
        "upstreamAccountName",
        "status",
        "failureKind",
        "failureClass",
    ] {
        require_nullable_text(object, field)?;
    }
    for field in [
        "sourceRowId",
        "occurredAtEpochMs",
        "upstreamAccountId",
        "inputTokens",
        "outputTokens",
        "cacheInputTokens",
        "reasoningTokens",
        "totalTokens",
    ] {
        require_nullable_i64(object, field)?;
    }
    for field in [
        "cost",
        "costInput",
        "costCacheWrite",
        "costCacheRead",
        "costOutput",
        "costReasoning",
        "tTotalMs",
        "tReqReadMs",
        "tReqParseMs",
        "tUpstreamConnectMs",
        "tUpstreamTtfbMs",
        "tUpstreamStreamMs",
        "firstTokenMs",
        "tRespParseMs",
    ] {
        require_nullable_finite(object, field)?;
    }
    require_bool(object, "successLike")?;
    require_bool(object, "terminal")?;
    require_nullable_bool(object, "isActionable")?;
    validate_filter_snapshot(required_object(object, "filterSnapshot")?)?;
    validate_deleted(object)?;
    Ok(())
}

fn validate_filter_snapshot(object: &Map<String, Value>) -> Result<()> {
    ensure_known_fields(object, FILTER_SNAPSHOT_FIELDS, "filterSnapshot")?;
    ensure!(
        required_i64(object, "version")? == SOURCE_FILTER_SNAPSHOT_VERSION,
        "unsupported filterSnapshot version"
    );
    for field in [
        "endpoint",
        "upstreamScope",
        "transport",
        "serviceTier",
        "serviceTierSnake",
        "proxyDisplayName",
        "stickyKey",
        "requesterIp",
        "promptCacheKey",
        "invokeId",
        "model",
        "failureKind",
        "errorMessage",
        "downstreamErrorMessage",
        "requestModel",
        "responseModel",
        "status",
        "failureClass",
        "reasoning",
        "upstreamAccountName",
        "reasoningEffort",
    ] {
        require_nullable_text(object, field)?;
    }
    require_nullable_i64(object, "upstreamAccountId")?;
    Ok(())
}

fn validate_pool_attempt(object: &Map<String, Value>) -> Result<()> {
    ensure_known_fields(object, POOL_ATTEMPT_FIELDS, "pool_attempt")?;
    ensure_fact_version(object, TypedSourceKind::PoolAttempt)?;
    for field in [
        "source",
        "attemptPublicId",
        "invokeId",
        "occurredAt",
        "endpoint",
        "requestModel",
        "upstreamRequestModel",
        "status",
        "phase",
        "failureKind",
        "proxyId",
        "proxyBindingKey",
        "finishedAt",
    ] {
        require_nullable_text(object, field)?;
    }
    for field in [
        "sourceRowId",
        "upstreamAccountId",
        "attemptIndex",
        "distinctAccountIndex",
        "sameAccountRetryIndex",
        "httpStatus",
        "downstreamHttpStatus",
    ] {
        require_nullable_i64(object, field)?;
    }
    for field in ["connectLatencyMs", "firstByteLatencyMs", "streamLatencyMs"] {
        require_nullable_nonnegative_finite(object, field)?;
    }
    let eligible = require_bool(object, "eligible")?;
    let binding = nullable_text(object, "proxyBindingKey")?;
    let finished_at = nullable_text(object, "finishedAt")?;
    let status = required_text(object, "status")?;
    let proxy_id = nullable_text(object, "proxyId")?;
    let expected_eligible = binding.is_some_and(|value| !value.is_empty())
        && finished_at.is_some_and(|value| !value.is_empty())
        && !status.eq_ignore_ascii_case("budget_exhausted_final");
    ensure!(
        eligible == expected_eligible,
        "pool attempt eligible flag does not match source qualification"
    );
    if eligible {
        ensure!(
            proxy_id.is_some_and(|value| !value.is_empty()),
            "eligible pool attempt must carry a stable proxy id"
        );
    }
    validate_deleted(object)?;
    Ok(())
}

fn validate_network_minute(object: &Map<String, Value>) -> Result<()> {
    ensure_known_fields(object, NETWORK_MINUTE_FIELDS, "network_minute")?;
    ensure_fact_version(object, TypedSourceKind::NetworkMinute)?;
    let bucket = required_i64(object, "bucketStartEpochSecond")?;
    ensure!(
        bucket >= 0 && bucket % 60 == 0,
        "network minute bucket is not aligned to a UTC minute"
    );
    require_nonempty_text(object, "source")?;
    require_nullable_text(object, "upstreamBaseUrlHost")?;
    require_nullable_i64(object, "upstreamAccountId")?;
    require_nonnegative_u64(object, "uploadBytes")?;
    require_nonnegative_u64(object, "downloadBytes")?;
    validate_deleted(object)?;
    Ok(())
}

fn validate_proxy_weight_hourly(object: &Map<String, Value>) -> Result<()> {
    ensure_known_fields(object, PROXY_WEIGHT_HOURLY_FIELDS, "proxy_weight_hourly")?;
    ensure_fact_version(object, TypedSourceKind::ProxyWeightHourly)?;
    require_nonempty_text(object, "proxyId")?;
    let bucket = required_i64(object, "bucketStartEpochSecond")?;
    ensure!(
        bucket >= 0 && bucket % 3600 == 0,
        "proxy weight bucket is not aligned to a UTC hour"
    );
    let sample_count = require_nonnegative_u64(object, "sampleCount")?;
    ensure!(
        sample_count > 0,
        "proxy weight sample count must be positive"
    );
    let min = required_finite(object, "minWeight")?;
    let max = required_finite(object, "maxWeight")?;
    let avg = required_finite(object, "avgWeight")?;
    let _last = required_finite(object, "lastWeight")?;
    ensure!(
        min <= avg && avg <= max,
        "proxy weight average is outside min/max"
    );
    require_nonnegative_u64(object, "lastSampleEpochUs")?;
    validate_deleted(object)?;
    Ok(())
}

fn validate_forward_proxy_attempt(object: &Map<String, Value>) -> Result<()> {
    ensure_known_fields(
        object,
        FORWARD_PROXY_ATTEMPT_FIELDS,
        "forward_proxy_attempt",
    )?;
    ensure_fact_version(object, TypedSourceKind::ForwardProxyAttempt)?;
    ensure!(
        require_nonempty_text(object, "source")? == "forward_proxy_attempts",
        "forward proxy attempt source must be forward_proxy_attempts"
    );
    ensure!(
        required_i64(object, "sourceRowId")? > 0,
        "forward proxy attempt source row id must be positive"
    );
    require_nonempty_text(object, "proxyKey")?;
    require_nonempty_text(object, "occurredAt")?;
    require_bool(object, "isSuccess")?;
    require_nullable_nonnegative_finite(object, "latencyMs")?;
    require_nullable_text(object, "failureKind")?;
    require_bool(object, "isProbe")?;
    validate_deleted(object)?;
    Ok(())
}

fn validate_deleted(object: &Map<String, Value>) -> Result<()> {
    if let Some(value) = object.get("deleted") {
        ensure!(
            value.as_bool().is_some(),
            "typed source deleted field must be boolean"
        );
    }
    Ok(())
}

fn ensure_fact_version(object: &Map<String, Value>, kind: TypedSourceKind) -> Result<()> {
    ensure!(
        required_i64(object, "factVersion")? == kind.fact_version(),
        "unexpected factVersion for typed source kind {}",
        kind.as_str()
    );
    Ok(())
}

fn ensure_known_fields(object: &Map<String, Value>, allowed: &[&str], kind: &str) -> Result<()> {
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        bail!("unknown {kind} typed source field: {unknown}");
    }
    for field in allowed.iter().filter(|field| **field != "deleted") {
        ensure!(
            object.contains_key(*field),
            "missing {kind} typed source field: {field}"
        );
    }
    Ok(())
}

fn required_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Map<String, Value>> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("{field} must be an object"))
}

fn required_i64(object: &Map<String, Value>, field: &str) -> Result<i64> {
    object
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("{field} must be an integer"))
}

fn require_nullable_i64(object: &Map<String, Value>, field: &str) -> Result<Option<i64>> {
    match object.get(field) {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{field} must be an integer or null")),
        None => bail!("missing field: {field}"),
    }
}

fn require_nonnegative_u64(object: &Map<String, Value>, field: &str) -> Result<u64> {
    let value = object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("{field} must be a non-negative integer"))?;
    ensure!(
        value <= i64::MAX as u64,
        "{field} exceeds the SQLite integer range"
    );
    Ok(value)
}

fn required_finite(object: &Map<String, Value>, field: &str) -> Result<f64> {
    let value = object
        .get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow::anyhow!("{field} must be a finite number"))?;
    ensure!(value.is_finite(), "{field} must be finite");
    Ok(value)
}

fn require_nullable_finite(object: &Map<String, Value>, field: &str) -> Result<Option<f64>> {
    match object.get(field) {
        Some(Value::Null) => Ok(None),
        Some(value) => {
            let value = value
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("{field} must be a finite number or null"))?;
            ensure!(value.is_finite(), "{field} must be finite");
            Ok(Some(value))
        }
        None => bail!("missing field: {field}"),
    }
}

fn require_nullable_nonnegative_finite(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<f64>> {
    let value = require_nullable_finite(object, field)?;
    if let Some(value) = value {
        ensure!(value >= 0.0, "{field} must not be negative");
    }
    Ok(value)
}

fn required_text<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{field} must be a non-empty string"))
}

fn require_nonempty_text<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str> {
    required_text(object, field)
}

fn nullable_text<'a>(object: &'a Map<String, Value>, field: &str) -> Result<Option<&'a str>> {
    match object.get(field) {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{field} must be a string or null")),
        None => bail!("missing field: {field}"),
    }
}

fn require_nullable_text<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>> {
    nullable_text(object, field)
}

fn require_bool(object: &Map<String, Value>, field: &str) -> Result<bool> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("{field} must be boolean"))
}

fn require_nullable_bool(object: &Map<String, Value>, field: &str) -> Result<Option<bool>> {
    match object.get(field) {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{field} must be boolean or null")),
        None => bail!("missing field: {field}"),
    }
}

fn opaque_entity_key(prefix: &str, tuple: &Value) -> Result<String> {
    let digest = digest_json(tuple)?;
    Ok(format!("{prefix}:sha256:{digest}"))
}

fn physical_identity(kind: TypedSourceKind, payload: &Value) -> Result<Value> {
    match kind {
        TypedSourceKind::NetworkMinute => Ok(json!([
            required_i64_value(payload, "bucketStartEpochSecond")?,
            required_text_value(payload, "source")?,
            nullable_text_value(payload, "upstreamBaseUrlHost")?,
            nullable_i64_value(payload, "upstreamAccountId")?,
        ])),
        TypedSourceKind::ProxyWeightHourly => Ok(json!([
            required_text_value(payload, "proxyId")?,
            required_i64_value(payload, "bucketStartEpochSecond")?,
        ])),
        _ => Ok(Value::Null),
    }
}

fn required_i64_value(payload: &Value, field: &str) -> Result<i64> {
    payload
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("{field} must be an integer"))
}

fn required_text_value<'a>(payload: &'a Value, field: &str) -> Result<&'a str> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{field} must be a string"))
}

fn required_nonempty_text_value<'a>(payload: &'a Value, field: &str) -> Result<&'a str> {
    let value = required_text_value(payload, field)?;
    ensure!(!value.is_empty(), "{field} must be a non-empty string");
    Ok(value)
}

fn nullable_i64_value(payload: &Value, field: &str) -> Result<Option<i64>> {
    match payload.get(field) {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{field} must be an integer or null")),
        None => bail!("missing field: {field}"),
    }
}

fn nullable_text_value<'a>(payload: &'a Value, field: &str) -> Result<Option<&'a str>> {
    match payload.get(field) {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{field} must be a string or null")),
        None => bail!("missing field: {field}"),
    }
}
