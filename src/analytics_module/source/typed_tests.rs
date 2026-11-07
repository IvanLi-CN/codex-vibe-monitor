use super::*;
use serde_json::{Value, json};
use sqlx::{Pool, Sqlite, sqlite::SqlitePoolOptions};

async fn test_pool() -> Pool<Sqlite> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect typed source test database");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("ensure typed source protocol schema");
    ensure_source_snapshot_schema(&pool)
        .await
        .expect("ensure typed source snapshot schema");
    pool
}

fn network_payload(host: Value, upload_bytes: u64) -> Value {
    json!({
        "factVersion": typed::SOURCE_NETWORK_MINUTE_FACT_VERSION,
        "bucketStartEpochSecond": 1_735_689_600_i64,
        "source": "proxy",
        "upstreamBaseUrlHost": host,
        "upstreamAccountId": Value::Null,
        "uploadBytes": upload_bytes,
        "downloadBytes": 20_u64,
    })
}

fn network_draft(
    event_id: &str,
    host: Value,
    upload_bytes: u64,
) -> typed::TypedSourceEnvelopeDraft {
    let payload = network_payload(host.clone(), upload_bytes);
    let host = host.as_str();
    typed::TypedSourceEnvelopeDraft::new(
        "source-instance",
        event_id,
        typed::TypedSourceKind::NetworkMinute,
        typed::network_minute_entity_key(1_735_689_600, "proxy", host, None)
            .expect("network entity key"),
        SourceOperation::Upsert,
        "2026-11-08T00:00:00Z",
        payload,
    )
}

#[test]
fn typed_validation_rejects_unknown_fields_without_truncating_text() {
    let mut payload = network_payload(Value::String("edge.example".to_string()), 10);
    payload["unexpected"] = json!(true);
    assert!(
        typed::validate_typed_source_payload(typed::TypedSourceKind::NetworkMinute, &payload)
            .expect_err("unknown typed field must fail closed")
            .to_string()
            .contains("unknown")
    );

    let long_error = "x".repeat(512);
    let mut filter_snapshot = serde_json::Map::new();
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
        filter_snapshot.insert(field.to_string(), Value::Null);
    }
    filter_snapshot.insert(
        "version".to_string(),
        json!(typed::SOURCE_FILTER_SNAPSHOT_VERSION),
    );
    filter_snapshot.insert(
        "errorMessage".to_string(),
        Value::String(long_error.clone()),
    );
    filter_snapshot.insert("upstreamAccountId".to_string(), Value::Null);
    let payload = json!({
        "factVersion": typed::SOURCE_INVOCATION_FACT_VERSION,
        "source": "proxy",
        "sourceRowId": 1,
        "invokeId": "invoke-1",
        "occurredAt": "2026-11-08T00:00:00Z",
        "occurredAtEpochMs": 1,
        "statsDate": "2026-11-08",
        "model": Value::Null,
        "requestModel": Value::Null,
        "responseModel": Value::Null,
        "reasoningEffort": Value::Null,
        "promptCacheKeyDigest": Value::Null,
        "upstreamAccountId": Value::Null,
        "upstreamAccountName": Value::Null,
        "status": "ok",
        "successLike": true,
        "terminal": true,
        "failureKind": Value::Null,
        "failureClass": Value::Null,
        "isActionable": Value::Null,
        "inputTokens": Value::Null,
        "outputTokens": Value::Null,
        "cacheInputTokens": Value::Null,
        "reasoningTokens": Value::Null,
        "totalTokens": Value::Null,
        "cost": Value::Null,
        "costInput": Value::Null,
        "costCacheWrite": Value::Null,
        "costCacheRead": Value::Null,
        "costOutput": Value::Null,
        "costReasoning": Value::Null,
        "tTotalMs": Value::Null,
        "tReqReadMs": Value::Null,
        "tReqParseMs": Value::Null,
        "tUpstreamConnectMs": Value::Null,
        "tUpstreamTtfbMs": Value::Null,
        "tUpstreamStreamMs": Value::Null,
        "firstTokenMs": Value::Null,
        "tRespParseMs": Value::Null,
        "filterSnapshot": Value::Object(filter_snapshot),
    });
    let validated =
        typed::validate_typed_source_payload(typed::TypedSourceKind::Invocation, &payload)
            .map(|_| payload.clone())
            .expect("validate complete invocation payload");
    assert_eq!(
        validated["filterSnapshot"]["errorMessage"],
        Value::String(long_error.clone())
    );
    let encoded = typed::typed_source_payload_encoded_size(&payload)
        .expect("measure canonical typed payload");
    assert!(encoded > long_error.len());
}

#[test]
fn network_entity_key_preserves_null_and_empty_host() {
    let null_host =
        typed::network_minute_entity_key(60, "proxy", None, None).expect("null host key");
    let empty_host =
        typed::network_minute_entity_key(60, "proxy", Some(""), None).expect("empty host key");
    let named_host =
        typed::network_minute_entity_key(60, "proxy", Some("edge"), None).expect("named host key");
    assert_ne!(null_host, empty_host);
    assert_ne!(empty_host, named_host);
    assert_ne!(null_host, named_host);
}

#[tokio::test]
async fn typed_append_is_atomic_and_retries_by_mutation_identity() {
    let pool = test_pool().await;
    let draft = network_draft("mutation-a", Value::String("edge".to_string()), 10);
    let mut tx = pool.begin().await.expect("begin typed append");
    let (first, first_receipt) = append_typed_source_envelope_tx(tx.as_mut(), &draft)
        .await
        .expect("append typed source");
    tx.commit().await.expect("commit typed append");
    assert_eq!(
        first.stats_semantics_version,
        typed::SOURCE_TYPED_STATS_SEMANTICS_VERSION
    );

    let mut tx = pool.begin().await.expect("begin typed retry");
    let (retry, retry_receipt) = append_typed_source_envelope_tx(tx.as_mut(), &draft)
        .await
        .expect("retry typed source");
    tx.commit().await.expect("commit typed retry");
    assert_eq!(retry_receipt, first_receipt);
    assert_eq!(retry.change_id, first.change_id);

    let legacy = SourceEnvelopeDraft {
        source_instance_id: "source-instance".to_string(),
        event_id: "legacy-mutation".to_string(),
        entity_kind: "legacy_fact".to_string(),
        entity_key: "legacy:1".to_string(),
        operation: SourceOperation::Upsert,
        occurred_at: "2026-11-08T00:00:01Z".to_string(),
        payload: json!({"factVersion": 1}),
    };
    let mut tx = pool.begin().await.expect("begin legacy append");
    append_source_envelope_tx(tx.as_mut(), &legacy)
        .await
        .expect("append legacy source");
    tx.commit().await.expect("commit legacy append");
    let legacy_stats: i64 = sqlx::query_scalar(
        "SELECT stats_semantics_version FROM analytics_change_events WHERE event_id = ?1",
    )
    .bind(&legacy.event_id)
    .fetch_one(&pool)
    .await
    .expect("load legacy stats semantics");
    assert_eq!(legacy_stats, SOURCE_STATS_SEMANTICS_VERSION);

    let mut connection = pool.acquire().await.expect("acquire");
    let reconciled = reconcile_typed_source_envelope_tx(&mut connection, &draft)
        .await
        .expect("reconcile typed source")
        .expect("committed mutation should reconcile");
    assert_eq!(reconciled.1, first_receipt);
}

#[tokio::test]
async fn different_mutations_with_the_same_payload_are_not_deduplicated() {
    let pool = test_pool().await;
    let first = network_draft("mutation-a", Value::String("edge".to_string()), 10);
    let second = network_draft("mutation-b", Value::String("edge".to_string()), 20);
    let third = network_draft("mutation-c", Value::String("edge".to_string()), 10);
    let mut tx = pool.begin().await.expect("begin A to B to A");
    append_typed_source_envelope_tx(tx.as_mut(), &first)
        .await
        .expect("append A");
    append_typed_source_envelope_tx(tx.as_mut(), &second)
        .await
        .expect("append B");
    append_typed_source_envelope_tx(tx.as_mut(), &third)
        .await
        .expect("append A again as a new mutation");
    tx.commit().await.expect("commit A to B to A");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM analytics_change_events WHERE entity_kind = 'network_minute'",
    )
    .fetch_one(&pool)
    .await
    .expect("count typed mutations");
    assert_eq!(count, 3);
}

#[test]
fn forward_proxy_attempt_has_an_independent_kind_and_identity() {
    let payload = json!({
        "factVersion": typed::SOURCE_FORWARD_PROXY_ATTEMPT_FACT_VERSION,
        "source": "forward_proxy_attempts",
        "sourceRowId": 7,
        "proxyKey": "proxy-a",
        "occurredAt": "2026-11-08T00:00:00Z",
        "isSuccess": true,
        "latencyMs": 0.0,
        "failureKind": Value::Null,
        "isProbe": false,
    });
    typed::validate_typed_source_payload(typed::TypedSourceKind::ForwardProxyAttempt, &payload)
        .expect("forward proxy attempt payload");
    assert_ne!(
        typed::forward_proxy_attempt_entity_key(7).expect("forward attempt key"),
        typed::pool_attempt_entity_key(7).expect("pool attempt key")
    );
}
