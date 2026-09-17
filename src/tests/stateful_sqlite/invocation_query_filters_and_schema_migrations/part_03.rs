#[tokio::test]
pub(crate) async fn list_invocations_status_success_excludes_resolved_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, status, error_message, failure_kind, failure_class) in [
        ("status-success-clean", Some("success"), None, None, None),
        (
            "status-success-trimmed",
            Some(" SUCCESS "),
            None,
            None,
            None,
        ),
        (
            "status-success-explicit-failure-class",
            Some("success"),
            Some("upstream exploded"),
            None,
            Some("service_failure"),
        ),
        (
            "status-success-legacy-failure-kind",
            Some("success"),
            Some("[upstream_response_failed] server_error"),
            Some("upstream_response_failed"),
            None,
        ),
        ("status-success-failed", Some("failed"), None, None, None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                failure_kind,
                failure_class,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 08:00:00")
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(error_message)
        .bind(failure_kind)
        .bind(failure_class)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert success status row");
    }

    let Json(success_filtered) = list_invocations(
        State(state),
        Query(ListQuery {
            status: Some("success".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("success status filter should succeed");

    let actual = success_filtered
        .records
        .into_iter()
        .map(|record| record.invoke_id)
        .collect::<HashSet<_>>();
    let expected = ["status-success-clean", "status-success-trimmed"]
        .into_iter()
        .map(String::from)
        .collect::<HashSet<_>>();
    assert_eq!(actual, expected);
}

#[tokio::test]
pub(crate) async fn list_invocations_warning_success_is_separate_from_success_and_failed() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_warning_success_invocations(&state.pool).await;

    let Json(warning_success_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            status: Some(INVOCATION_STATUS_WARNING_SUCCESS.to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("warning success status filter should succeed");
    assert_eq!(warning_success_filtered.total, 1);
    assert_eq!(
        warning_success_filtered.records[0].invoke_id,
        "status-warning-success"
    );

    let Json(success_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            status: Some("success".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("success status filter should stay separate");
    assert_eq!(success_filtered.total, 1);
    assert_eq!(
        success_filtered.records[0].invoke_id,
        "status-plain-success"
    );

    let Json(failed_filtered) = list_invocations(
        State(state),
        Query(ListQuery {
            status: Some("failed".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("failed status filter should exclude warning success");
    assert_eq!(failed_filtered.total, 1);
    assert_eq!(
        failed_filtered.records[0].invoke_id,
        "status-historical-downstream-failed"
    );
}

async fn seed_warning_success_invocations(pool: &Pool<Sqlite>) {
    for (invoke_id, status, failure_kind, failure_class, downstream_error_message) in [
        ("status-plain-success", "success", None, None, None),
        (
            "status-warning-success",
            INVOCATION_STATUS_WARNING_SUCCESS,
            Some("downstream_closed"),
            Some("none"),
            Some("[downstream_closed] downstream closed while streaming upstream response"),
        ),
        (
            "status-historical-downstream-failed",
            "failed",
            Some("downstream_closed"),
            Some("client_abort"),
            Some("[downstream_closed] downstream closed while streaming upstream response"),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                failure_kind,
                failure_class,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 08:00:00")
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(failure_kind)
        .bind(failure_class)
        .bind(downstream_error_message.map(|message| {
            json!({
                "downstreamErrorMessage": message,
            })
            .to_string()
        }))
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert warning success filter row");
    }
}

#[tokio::test]
pub(crate) async fn list_invocations_status_sort_uses_normalized_status_values() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, occurred_at, status, error_message, failure_kind) in [
        (
            "status-sort-trimmed-success",
            "2026-03-10 08:02:00",
            Some(" SUCCESS "),
            None,
            None,
        ),
        (
            "status-sort-success",
            "2026-03-10 08:01:00",
            Some("success"),
            None,
            None,
        ),
        (
            "status-sort-failed",
            "2026-03-10 08:03:00",
            Some("failed"),
            None,
            None,
        ),
        (
            "status-sort-legacy-success-failure",
            "2026-03-10 08:04:00",
            Some("success"),
            Some("[upstream_response_failed] server_error"),
            Some("upstream_response_failed"),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                failure_kind,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(error_message)
        .bind(failure_kind)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert status sort row");
    }

    let Json(sorted) = list_invocations(
        State(state),
        Query(ListQuery {
            page: Some(1),
            page_size: Some(20),
            sort_by: Some("status".to_string()),
            sort_order: Some("asc".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("status sort should succeed");

    let actual = sorted
        .records
        .into_iter()
        .map(|record| record.invoke_id)
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            "status-sort-legacy-success-failure".to_string(),
            "status-sort-failed".to_string(),
            "status-sort-trimmed-success".to_string(),
            "status-sort-success".to_string(),
        ]
    );
}

#[tokio::test]
pub(crate) async fn fetch_invocation_summary_reports_new_records_count_for_applied_filters() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    insert_summary_seed(
        &state.pool,
        SummarySeed {
            invoke_id: "summary-base",
            occurred_at: "2026-03-10 09:00:00",
            model: "gpt-5.4",
            total_tokens: 120,
            cache_input_tokens: 20,
            cost: 0.012,
            status: "success",
            ttfb_ms: 100.0,
            total_ms: 250.0,
        },
    )
    .await;

    let Json(initial_list) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            model: Some("gpt-5.4".to_string()),
            page: Some(1),
            page_size: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("seed list query should succeed");

    for (invoke_id, model) in [
        ("summary-new-match", "gpt-5.4"),
        ("summary-new-other", "gpt-5.3-codex"),
    ] {
        insert_summary_seed(
            &state.pool,
            SummarySeed {
                invoke_id,
                occurred_at: "2026-03-10 09:05:00",
                model,
                total_tokens: 240,
                cache_input_tokens: 40,
                cost: 0.024,
                status: "failed",
                ttfb_ms: 180.0,
                total_ms: 500.0,
            },
        )
        .await;
    }

    let Json(summary) = fetch_invocation_summary(
        State(state),
        Query(ListQuery {
            model: Some("gpt-5.4".to_string()),
            snapshot_id: Some(initial_list.snapshot_id),
            ..Default::default()
        }),
    )
    .await
    .expect("summary query should succeed");

    assert_eq!(summary.snapshot_id, initial_list.snapshot_id);
    assert_eq!(summary.new_records_count, 1);
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.token.request_count, 1);
    assert_eq!(summary.token.total_tokens, 120);
    assert_eq!(summary.token.cache_input_tokens, 20);
    assert_f64_close(summary.token.avg_tokens_per_request, 120.0);
    assert_f64_close(summary.network.avg_ttfb_ms.unwrap_or_default(), 100.0);
    assert_f64_close(summary.network.p95_total_ms.unwrap_or_default(), 250.0);
    assert_eq!(summary.exception.failure_count, 0);
}

struct SummarySeed<'a> {
    invoke_id: &'a str,
    occurred_at: &'a str,
    model: &'a str,
    total_tokens: i64,
    cache_input_tokens: i64,
    cost: f64,
    status: &'a str,
    ttfb_ms: f64,
    total_ms: f64,
}

async fn insert_summary_seed(pool: &Pool<Sqlite>, seed: SummarySeed<'_>) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            total_tokens,
            cache_input_tokens,
            cost,
            status,
            t_upstream_ttfb_ms,
            t_total_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(seed.invoke_id)
    .bind(seed.occurred_at)
    .bind(SOURCE_PROXY)
    .bind(seed.model)
    .bind(seed.total_tokens)
    .bind(seed.cache_input_tokens)
    .bind(seed.cost)
    .bind(seed.status)
    .bind(seed.ttfb_ms)
    .bind(seed.total_ms)
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert summary seed row");
}

#[tokio::test]
pub(crate) async fn fetch_invocation_summary_resolves_failure_class_for_legacy_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_legacy_failure_class_invocations(&state.pool).await;

    let Json(summary) = fetch_invocation_summary(State(state), Query(ListQuery::default()))
        .await
        .expect("summary query should succeed");

    assert_eq!(summary.total_count, 7);
    assert_eq!(summary.failure_count, 5);
    assert_eq!(summary.exception.failure_count, 5);
    assert_eq!(summary.exception.service_failure_count, 3);
    assert_eq!(summary.exception.client_failure_count, 1);
    assert_eq!(summary.exception.client_abort_count, 1);
    assert_eq!(summary.exception.actionable_failure_count, 3);
}

async fn seed_legacy_failure_class_invocations(pool: &Pool<Sqlite>) {
    for (invoke_id, status, error_message) in [
        ("legacy-service", "failed", None),
        ("legacy-client", "http_401", None),
        (
            "legacy-abort",
            "failed",
            Some("[downstream_closed] user cancelled"),
        ),
        ("legacy-running", "running", None),
        ("legacy-pending", "pending", None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 09:00:00")
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(error_message)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert legacy failure row");
    }

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            failure_kind,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("legacy-429")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind("upstream_http_429")
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert legacy 429 row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            failure_kind,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("legacy-stream-success")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("[upstream_response_failed] server_error")
    .bind("upstream_response_failed")
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert legacy stream failure row");
}

#[tokio::test]
pub(crate) async fn fetch_invocation_summary_normalizes_top_level_success_and_failure_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_normalized_summary_statuses(&state.pool).await;

    let Json(summary) = fetch_invocation_summary(State(state), Query(ListQuery::default()))
        .await
        .expect("summary query should succeed");

    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 2);
    assert_eq!(summary.exception.failure_count, 2);
    assert_eq!(summary.exception.service_failure_count, 2);
}

async fn seed_normalized_summary_statuses(pool: &Pool<Sqlite>) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("summary-success-trimmed")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind(" SUCCESS ")
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert trimmed success row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("summary-http-200-success")
    .bind("2026-03-10 09:00:30")
    .bind(SOURCE_PROXY)
    .bind("http_200")
    .bind("")
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert legacy http_200 success row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            error_message,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("summary-null-status-failure")
    .bind("2026-03-10 09:01:00")
    .bind(SOURCE_PROXY)
    .bind("upstream exploded")
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert null-status failure row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            failure_kind,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("summary-legacy-success-failure")
    .bind("2026-03-10 09:02:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("[upstream_response_failed] server_error")
    .bind("upstream_response_failed")
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert legacy success failure row");
}

#[tokio::test]
pub(crate) async fn fetch_invocation_summary_keeps_zero_ms_ttft_and_excludes_negative_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            t_upstream_ttfb_ms,
            first_token_ms,
            t_upstream_stream_ms,
            t_total_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("summary-zero-network")
    .bind("2026-03-10 09:10:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(0.0_f64)
    .bind(0.0_f64)
    .bind(0.0_f64)
    .bind(0.0_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert zero-ms summary row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            first_token_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("summary-negative-first-token")
    .bind("2026-03-10 09:11:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(-1.0_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert invalid TTFT summary row");

    let Json(summary) = fetch_invocation_summary(State(state), Query(ListQuery::default()))
        .await
        .expect("summary query with zero-ms samples should succeed");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.network.avg_ttfb_ms, Some(0.0));
    assert_eq!(summary.network.p95_ttfb_ms, Some(0.0));
    assert_eq!(summary.network.avg_first_token_ms, Some(0.0));
    assert_eq!(summary.network.p95_first_token_ms, Some(0.0));
    assert_eq!(summary.network.avg_response_duration_ms, None);
    assert_eq!(summary.network.p95_response_duration_ms, None);
    assert_eq!(summary.network.avg_total_ms, Some(0.0));
    assert_eq!(summary.network.p95_total_ms, Some(0.0));
}

#[tokio::test]
pub(crate) async fn list_invocations_keeps_negative_ttft_in_requesting_phase() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            first_token_ms,
            t_upstream_connect_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("running-negative-first-token")
    .bind("2026-03-10 09:12:00")
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(-1.0_f64)
    .bind(120.0_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert running invalid TTFT row");

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            invoke_id: Some("running-negative-first-token".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("list query with invalid running TTFT should succeed");

    assert_eq!(response.total, 1);
    assert_eq!(
        response.records[0].live_phase.as_deref(),
        Some("requesting")
    );
}

#[tokio::test]
pub(crate) async fn persisted_nonfinite_timing_is_unavailable_to_phase_and_summary_queries() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            t_upstream_ttfb_ms,
            first_token_ms,
            t_upstream_stream_ms,
            t_total_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("summary-infinite-timing")
    .bind("2026-03-10 09:13:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(f64::INFINITY)
    .bind(f64::INFINITY)
    .bind(f64::INFINITY)
    .bind(f64::INFINITY)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert nonfinite timing row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            first_token_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("running-infinite-first-token")
    .bind("2026-03-10 09:14:00")
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(f64::INFINITY)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert running nonfinite TTFT row");

    let Json(summary) = fetch_invocation_summary(State(state.clone()), Query(ListQuery::default()))
        .await
        .expect("summary query with nonfinite timing should succeed");

    assert_eq!(summary.network.avg_ttfb_ms, None);
    assert_eq!(summary.network.p95_ttfb_ms, None);
    assert_eq!(summary.network.avg_first_token_ms, None);
    assert_eq!(summary.network.p95_first_token_ms, None);
    assert_eq!(summary.network.avg_response_duration_ms, None);
    assert_eq!(summary.network.p95_response_duration_ms, None);
    assert_eq!(summary.network.avg_total_ms, None);
    assert_eq!(summary.network.p95_total_ms, None);

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            invoke_id: Some("running-infinite-first-token".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("list query with nonfinite running TTFT should succeed");

    assert_eq!(response.total, 1);
    assert_eq!(response.records[0].live_phase.as_deref(), Some("queued"));
}

#[tokio::test]
pub(crate) async fn fetch_invocation_summary_returns_zero_values_for_empty_results() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(summary) = fetch_invocation_summary(
        State(state),
        Query(ListQuery {
            model: Some("missing-model".to_string()),
            snapshot_id: Some(999),
            ..Default::default()
        }),
    )
    .await
    .expect("empty summary query should succeed");

    assert_eq!(summary.snapshot_id, 999);
    assert_eq!(summary.new_records_count, 0);
    assert_eq!(summary.total_count, 0);
    assert_eq!(summary.success_count, 0);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 0);
    assert_f64_close(summary.total_cost, 0.0);
    assert_eq!(summary.token.request_count, 0);
    assert_eq!(summary.token.total_tokens, 0);
    assert_f64_close(summary.token.avg_tokens_per_request, 0.0);
    assert_eq!(summary.token.cache_input_tokens, 0);
    assert_f64_close(summary.token.total_cost, 0.0);
    assert_eq!(summary.network.avg_ttfb_ms, None);
    assert_eq!(summary.network.p95_ttfb_ms, None);
    assert_eq!(summary.network.avg_first_token_ms, None);
    assert_eq!(summary.network.p95_first_token_ms, None);
    assert_eq!(summary.network.avg_response_duration_ms, None);
    assert_eq!(summary.network.p95_response_duration_ms, None);
    assert_eq!(summary.network.avg_total_ms, None);
    assert_eq!(summary.network.p95_total_ms, None);
    assert_eq!(summary.exception.failure_count, 0);
    assert_eq!(summary.exception.service_failure_count, 0);
    assert_eq!(summary.exception.client_failure_count, 0);
    assert_eq!(summary.exception.client_abort_count, 0);
    assert_eq!(summary.exception.actionable_failure_count, 0);
}

#[tokio::test]
pub(crate) async fn fetch_invocation_summary_p95_ignores_zero_response_duration_placeholders() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, stream_ms) in [
        ("summary-zero-response", 0.0_f64),
        ("summary-positive-response", 100.0_f64),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, t_upstream_stream_ms, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 09:10:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(stream_ms)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert response-duration summary row");
    }

    let Json(summary) = fetch_invocation_summary(State(state), Query(ListQuery::default()))
        .await
        .expect("summary query with response-duration samples should succeed");

    assert_eq!(summary.network.avg_response_duration_ms, Some(100.0));
    assert_eq!(summary.network.p95_response_duration_ms, Some(100.0));
}

#[tokio::test]
pub(crate) async fn fetch_invocation_summary_ignores_stale_retry_timing() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, first_token_ms,
            t_upstream_stream_ms, raw_response
        ) VALUES (?1, ?2, ?3, 'failed', ?4, ?5, '{}')
        "#,
    )
    .bind("summary-stale-retry-timing")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(900.0_f64)
    .bind(800.0_f64)
    .execute(&state.pool)
    .await
    .expect("insert stale retry invocation");
    for (public_id, index, status, stream_ms, first_byte_ms) in [
        ("STALE1", 1_i64, "failed", Some(800.0_f64), Some(100.0_f64)),
        ("STALE2", 2_i64, "failed", None, None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                attempt_public_id, invoke_id, occurred_at, endpoint, route_mode,
                attempt_index, distinct_account_index, same_account_retry_index,
                requester_ip, started_at, finished_at, status, phase,
                stream_latency_ms, first_byte_latency_ms, created_at
            ) VALUES (?1, ?2, ?3, '/v1/responses', 'pool', ?4, 1, 0,
                      '127.0.0.1', ?3, ?3, ?5, 'completed', ?6, ?7, ?3)
            "#,
        )
        .bind(public_id)
        .bind("summary-stale-retry-timing")
        .bind(&occurred_at)
        .bind(index)
        .bind(status)
        .bind(stream_ms)
        .bind(first_byte_ms)
        .execute(&state.pool)
        .await
        .expect("insert retry attempt timing");
    }

    let Json(summary) = fetch_invocation_summary(State(state), Query(ListQuery::default()))
        .await
        .expect("summary should ignore stale retry timing");
    assert_eq!(summary.network.avg_first_token_ms, None);
    assert_eq!(summary.network.p95_first_token_ms, None);
    assert_eq!(summary.network.avg_response_duration_ms, None);
    assert_eq!(summary.network.p95_response_duration_ms, None);
}

use super::*;
