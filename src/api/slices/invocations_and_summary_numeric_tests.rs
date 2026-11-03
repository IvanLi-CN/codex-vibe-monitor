use super::*;

#[test]
fn summary_live_wait_arithmetic_fails_closed_on_non_finite_values() {
    assert!(matches!(
        summary_live_sum_from_average(Some(f64::MAX), 2),
        Err(ApiError::Unavailable(_))
    ));

    let mut sum = f64::MAX;
    assert!(matches!(
        checked_summary_live_add(&mut sum, f64::MAX, "test wait sum"),
        Err(ApiError::Unavailable(_))
    ));

    let mut rounded = 0.1;
    checked_summary_live_subtract(&mut rounded, 0.1, "test wait subtraction")
        .expect("equal values should subtract cleanly");
    assert_eq!(rounded, 0.0);

    let mut inconsistent = 0.1;
    assert!(matches!(
        checked_summary_live_subtract(&mut inconsistent, 4.0, "test wait subtraction"),
        Err(ApiError::Unavailable(_))
    ));
}

#[test]
fn runtime_summary_overlay_rejects_negative_cache_write_tokens() {
    let mut record = invocation_cost_audit_tests::sample_invocation(None);
    record.cache_write_tokens = Some(-1);
    let mut delta = RuntimeSummaryOverlayDelta::default();

    assert!(matches!(
        delta.add_terminal_record(&record),
        Err(ApiError::Unavailable(_))
    ));
}

#[test]
fn projection_sql_keeps_success_aliases_and_invocation_identity() {
    assert!(INVOCATION_RESOLVED_FAILURE_CLASS_SQL.contains("'succeeded'"));
    assert!(INVOCATION_RESOLVED_FAILURE_CLASS_SQL.contains("'ok'"));
    let sql = invocation_upstream_account_id_from_attempt_fallback_sql("inv");
    assert!(sql.contains("julianday(attempt.occurred_at"));
    assert!(sql.contains("julianday(inv.occurred_at"));
}

#[tokio::test]
async fn attempt_account_fallback_ignores_invalid_payload_account_values() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect account fallback test database");
    sqlx::query(
        "CREATE TABLE codex_invocations (invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, payload TEXT)",
    )
    .execute(&pool)
    .await
    .expect("create invocation fallback table");
    sqlx::query(
        "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, attempt_index INTEGER NOT NULL, upstream_account_id INTEGER)",
    )
    .execute(&pool)
    .await
    .expect("create attempt fallback table");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, payload) VALUES ('invoke-fallback', '2026-10-31T00:00:00Z', '{\"upstreamAccountId\":\"invalid\"}')",
    )
    .execute(&pool)
    .await
    .expect("insert invalid invocation account");
    sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, attempt_index, upstream_account_id) VALUES (1, 'invoke-fallback', '2026-10-31T00:00:00.000Z', 1, 42)",
    )
    .execute(&pool)
    .await
    .expect("insert fallback account");

    let sql = invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let account_id =
        sqlx::query_scalar::<_, Option<i64>>(&format!("SELECT {sql} FROM codex_invocations"))
            .fetch_one(&pool)
            .await
            .expect("evaluate account fallback");
    assert_eq!(account_id, Some(42));
}

#[tokio::test]
async fn projection_sql_resolves_legacy_none_and_unknown_errors_like_rust() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect resolver test database");
    sqlx::query(
        "CREATE TABLE codex_invocations (status TEXT, error_message TEXT, failure_kind TEXT, failure_class TEXT, payload TEXT)",
    )
    .execute(&pool)
    .await
    .expect("create resolver test table");
    sqlx::query(
        r#"INSERT INTO codex_invocations (status, error_message, failure_kind, failure_class, payload) VALUES
         ('completed', '', 'none', NULL, '{}'),
         ('completed', 'opaque upstream error', 'none', NULL, '{}'),
         ('warning_success', '', 'none', NULL, '{}'),
         ('completed', '', 'unknown_legacy_kind', NULL, '{}'),
         ('completed', '', 'none', NULL, '{"failureKind":"upstream_stream_error"}'),
         ('completed', '', 'upstream_stream_error', NULL, '{"failureKind":"none"}'),
         ('completed', '', 'upstream_stream_error', NULL, '{"failureKind":""}'),
         ('completed', '', 'none', NULL, '{"downstreamErrorMessage":"downstream closed while streaming upstream response"}'),
         ('completed', '', 'none', NULL, '{"failureKind":0}'),
         ('completed', '', 'none', NULL, '{"downstreamErrorMessage":false}'),
         ('completed', '', 'none', NULL, '{"downstreamErrorMessage":"upstream stream error"}'),
         ('completed', '[invalid_api_key] invalid key', 'none', NULL, '{}')"#,
    )
    .execute(&pool)
    .await
    .expect("insert resolver fixtures");

    let sql = format!("SELECT {INVOCATION_RESOLVED_FAILURE_CLASS_SQL} FROM codex_invocations");
    let classes = sqlx::query_scalar::<_, String>(&sql)
        .fetch_all(&pool)
        .await
        .expect("evaluate resolver expression");
    assert_eq!(
        classes,
        [
            "none",
            "service_failure",
            "none",
            "service_failure",
            "service_failure",
            "none",
            "service_failure",
            "client_abort",
            "none",
            "none",
            "service_failure",
            "client_failure"
        ]
    );

    let kinds = sqlx::query_scalar::<_, Option<String>>(&format!(
        "SELECT {INVOCATION_FAILURE_KIND_SQL} FROM codex_invocations"
    ))
    .fetch_all(&pool)
    .await
    .expect("evaluate normalized failure kind expression");
    assert_eq!(
        kinds,
        [
            None,
            Some("untyped_failure".to_string()),
            None,
            Some("unknown_legacy_kind".to_string()),
            Some("upstream_stream_error".to_string()),
            None,
            Some("upstream_stream_error".to_string()),
            Some("downstream_closed".to_string()),
            None,
            None,
            Some("upstream_stream_error".to_string()),
            Some("invalid_api_key".to_string())
        ]
    );

    let fixtures = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT status, error_message, failure_kind, failure_class, payload FROM codex_invocations",
    )
    .fetch_all(&pool)
    .await
    .expect("load resolver fixtures for Rust parity");
    let expected = fixtures
        .iter()
        .map(
            |(status, error_message, failure_kind, failure_class, payload)| {
                let payload =
                    crate::analytics_changes::normalize_analytics_payload(payload.as_deref());
                let failure_kind = crate::analytics_changes::normalize_analytics_failure_kind(
                    payload.as_ref(),
                    failure_kind.as_deref(),
                );
                let classification_error_message =
                    crate::analytics_changes::normalize_analytics_classification_error_message(
                        payload.as_ref(),
                        error_message.as_deref(),
                    );
                let classification = resolve_failure_classification(
                    status.as_deref(),
                    classification_error_message.as_deref(),
                    failure_kind.as_deref(),
                    failure_class.as_deref(),
                    None,
                );
                (
                    classification.failure_class.as_str().to_string(),
                    classification.failure_kind,
                )
            },
        )
        .collect::<Vec<_>>();
    assert_eq!(classes.into_iter().zip(kinds).collect::<Vec<_>>(), expected);
}
