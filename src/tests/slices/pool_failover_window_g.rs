#[tokio::test]
async fn backfill_invocation_service_tiers_revisits_inline_proxy_auto_tiers_without_raw_files() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-inline-service-tier-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses","serviceTier":"auto"}"#)
    .bind(r#"{"service_tier":"default"}"#)
    .execute(&pool)
    .await
    .expect("insert inline proxy service tier row");

    let summary_first = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("inline service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-inline-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query inline proxy payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode inline payload JSON");
    assert_eq!(payload_json["serviceTier"], "default");
    assert_eq!(
        payload_json["serviceTierBackfillVersion"],
        "stream-terminal-v1"
    );

    let summary_second = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("inline service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);
}

#[tokio::test]
async fn backfill_invocation_service_tiers_revisits_inline_proxy_non_auto_stream_tiers() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-inline-non-auto-service-tier-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses","serviceTier":"priority"}"#)
    .bind(
        [
            "event: response.created",
            r#"data: {"type":"response.created","response":{"service_tier":"priority"}}"#,
            "",
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{"service_tier":"default"}}"#,
            "",
        ]
        .join("\n"),
    )
    .execute(&pool)
    .await
    .expect("insert inline proxy non-auto service tier row");

    let summary_first = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("inline non-auto service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-inline-non-auto-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query inline proxy non-auto payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode inline non-auto payload JSON");
    assert_eq!(payload_json["serviceTier"], "default");
    assert_eq!(
        payload_json["serviceTierBackfillVersion"],
        "stream-terminal-v1"
    );

    let summary_second = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("inline non-auto service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);
}

#[tokio::test]
async fn backfill_invocation_service_tiers_tracks_skip_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("service-tier-missing")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_XY)
    .bind("success")
    .bind("{}")
    .bind(r#"{"status":"success"}"#)
    .execute(&pool)
    .await
    .expect("insert missing tier row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("service-tier-missing-file")
    .bind("2026-02-23 00:00:01")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses"}"#)
    .bind("{}")
    .bind("/tmp/does-not-exist-response.bin")
    .execute(&pool)
    .await
    .expect("insert missing file row");

    let summary = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("service tier backfill skip run should succeed");
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_missing_tier, 1);
}

#[tokio::test]
async fn backfill_proxy_usage_tokens_updates_missing_tokens_idempotently() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = std::env::temp_dir().join(format!(
        "proxy-usage-backfill-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let response_path = temp_dir.join("response.bin");
    let raw = [
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":88,\"output_tokens\":22,\"total_tokens\":110,\"input_tokens_details\":{\"cached_tokens\":9},\"output_tokens_details\":{\"reasoning_tokens\":3}}}}",
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");
    fs::write(&response_path, compressed).expect("write response payload");

    let row_count = BACKFILL_BATCH_SIZE as usize + 5;
    for index in 0..row_count {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(format!("proxy-backfill-test-{index}"))
        .bind("2026-02-23 00:00:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(
            "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":true,\"requestModel\":null,\"responseModel\":null,\"usageMissingReason\":null,\"requestParseError\":null}",
        )
        .bind("{}")
        .bind(response_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("insert proxy row");
    }

    let summary_first = backfill_proxy_usage_tokens(&pool, None)
        .await
        .expect("first backfill should succeed");
    assert_eq!(summary_first.scanned, row_count as u64);
    assert_eq!(summary_first.updated, row_count as u64);

    let row = sqlx::query(
        r#"
        SELECT
          COUNT(*) AS total_rows,
          SUM(CASE WHEN input_tokens = 88 THEN 1 ELSE 0 END) AS input_tokens_88,
          SUM(CASE WHEN output_tokens = 22 THEN 1 ELSE 0 END) AS output_tokens_22,
          SUM(CASE WHEN cache_input_tokens = 9 THEN 1 ELSE 0 END) AS cache_input_tokens_9,
          SUM(CASE WHEN reasoning_tokens = 3 THEN 1 ELSE 0 END) AS reasoning_tokens_3,
          SUM(CASE WHEN total_tokens = 110 THEN 1 ELSE 0 END) AS total_tokens_110
        FROM codex_invocations
        WHERE source = ?1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("fetch backfilled rows");
    assert_eq!(
        row.try_get::<i64, _>("total_rows")
            .expect("read total_rows"),
        row_count as i64
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("input_tokens_88")
            .expect("read input_tokens_88"),
        Some(row_count as i64)
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("output_tokens_22")
            .expect("read output_tokens_22"),
        Some(row_count as i64)
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cache_input_tokens_9")
            .expect("read cache_input_tokens_9"),
        Some(row_count as i64)
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("reasoning_tokens_3")
            .expect("read reasoning_tokens_3"),
        Some(row_count as i64)
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("total_tokens_110")
            .expect("read total_tokens_110"),
        Some(row_count as i64)
    );

    let summary_second = backfill_proxy_usage_tokens(&pool, None)
        .await
        .expect("second backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_usage_tokens_reads_from_fallback_root_for_relative_paths() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-usage-backfill-fallback");
    let fallback_root = temp_dir.join("legacy-root");
    let relative_path = PathBuf::from("proxy_raw_payloads/response-fallback.bin");
    let response_path = fallback_root.join(&relative_path);
    let response_dir = response_path.parent().expect("response parent dir");
    fs::create_dir_all(response_dir).expect("create fallback response dir");
    write_backfill_response_payload(&response_path);

    insert_proxy_backfill_row(&pool, "proxy-usage-backfill-fallback", &relative_path).await;
    let row_id: i64 = sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
        .bind("proxy-usage-backfill-fallback")
        .fetch_one(&pool)
        .await
        .expect("query fallback row id");

    let summary = backfill_proxy_usage_tokens_up_to_id(&pool, row_id, Some(&fallback_root))
        .await
        .expect("usage backfill with fallback root should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 0);
    assert_eq!(summary.skipped_without_usage, 0);
    assert_eq!(summary.skipped_decode_error, 0);

    let total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-usage-backfill-fallback")
            .fetch_one(&pool)
            .await
            .expect("query fallback usage row");
    assert_eq!(total_tokens, Some(110));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_usage_tokens_respects_snapshot_upper_bound() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-usage-backfill-snapshot");
    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload(&response_path);

    let first_invoke_id = "proxy-backfill-snapshot-first";
    let second_invoke_id = "proxy-backfill-snapshot-second";
    insert_proxy_backfill_row(&pool, first_invoke_id, &response_path).await;
    insert_proxy_backfill_row(&pool, second_invoke_id, &response_path).await;

    let first_id: i64 = sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
        .bind(first_invoke_id)
        .fetch_one(&pool)
        .await
        .expect("query first id");
    let second_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
            .bind(second_invoke_id)
            .fetch_one(&pool)
            .await
            .expect("query second id");

    let summary_first = backfill_proxy_usage_tokens_up_to_id(&pool, first_id, None)
        .await
        .expect("backfill up to first id should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);

    let first_total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind(first_invoke_id)
            .fetch_one(&pool)
            .await
            .expect("query first row tokens");
    let second_total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind(second_invoke_id)
            .fetch_one(&pool)
            .await
            .expect("query second row tokens");
    assert_eq!(first_total_tokens, Some(110));
    assert_eq!(second_total_tokens, None);

    let summary_second = backfill_proxy_usage_tokens_up_to_id(&pool, second_id, None)
        .await
        .expect("backfill up to second id should succeed");
    assert_eq!(summary_second.scanned, 1);
    assert_eq!(summary_second.updated, 1);

    let second_total_tokens_after: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind(second_invoke_id)
            .fetch_one(&pool)
            .await
            .expect("query second row tokens after second backfill");
    assert_eq!(second_total_tokens_after, Some(110));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_missing_costs_updates_dated_model_alias_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-dated-model",
        Some("gpt-5.2-2025-12-11"),
        Some(1_000),
        Some(500),
    )
    .await;

    let catalog = PricingCatalog {
        version: "unit-cost-backfill".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary_first = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("first cost backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_unpriced_model, 0);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-cost-backfill-dated-model")
    .fetch_one(&pool)
    .await
    .expect("query updated cost row");
    let expected = ((1_000.0 * 2.0) + (500.0 * 3.0)) / 1_000_000.0;
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read cost")
            .expect("cost should exist")
            - expected)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cost_estimated")
            .expect("read cost_estimated"),
        Some(1)
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read price_version")
            .as_deref(),
        Some("unit-cost-backfill@response-tier")
    );

    let summary_second = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("second cost backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);
}

#[tokio::test]
async fn backfill_proxy_missing_costs_backfills_standard_rows_with_missing_billing_service_tier() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-standard-null-billing-tier")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.2")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.0035_f64)
    .bind(1_i64)
    .bind("unit-cost-backfill")
    .bind(r#"{"endpoint":"/v1/responses","serviceTier":"default","billingServiceTier":null}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert standard proxy cost row");

    let catalog = PricingCatalog {
        version: "unit-cost-backfill".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("standard missing billing tier row should be backfilled");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_unpriced_model, 0);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-standard-null-billing-tier")
    .fetch_one(&pool)
    .await
    .expect("query standard proxy cost row");

    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read standard cost")
            .expect("standard cost should exist")
            - 0.0035)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cost_estimated")
            .expect("read standard cost_estimated"),
        Some(1)
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read standard price_version")
            .as_deref(),
        Some("unit-cost-backfill@response-tier")
    );

    let payload: String = row.try_get("payload").expect("read standard payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode standard payload JSON");
    assert_eq!(payload_json["serviceTier"], "default");
    assert_eq!(payload_json["billingServiceTier"], "default");

    let summary_second = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("standard row backfill should become idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);
}

#[tokio::test]
async fn backfill_proxy_missing_costs_rewrites_stale_standard_billing_service_tier() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-standard-stale-billing-tier")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.2")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.0035_f64)
    .bind(1_i64)
    .bind("unit-cost-backfill")
    .bind(r#"{"endpoint":"/v1/responses","serviceTier":"default","billingServiceTier":"auto"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert stale standard billing tier row");

    let catalog = PricingCatalog {
        version: "unit-cost-backfill".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("stale standard billing tier row should be backfilled");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_unpriced_model, 0);

    let row = sqlx::query("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
        .bind("proxy-standard-stale-billing-tier")
        .fetch_one(&pool)
        .await
        .expect("query stale standard billing tier row");

    let payload: String = row.try_get("payload").expect("read stale standard payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode stale standard payload JSON");
    assert_eq!(payload_json["serviceTier"], "default");
    assert_eq!(payload_json["billingServiceTier"], "default");

    let summary_second = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("stale standard billing tier row should become idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);
}

#[tokio::test]
async fn backfill_proxy_missing_costs_reprices_api_keys_requested_tier_rows() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let created_at = "2026-01-01T00:00:00Z".to_string();
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, upstream_base_url, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(2568_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("API Keys Pool")
    .bind("https://api-keys.vendor.invalid/")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert api keys upstream account");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-api-keys-requested-tier-cost-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":2568,"upstreamAccountName":"API Keys Pool","routeMode":"pool"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert api keys proxy cost row");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("api keys requested-tier cost backfill should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_unpriced_model, 0);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-api-keys-requested-tier-cost-backfill")
    .fetch_one(&pool)
    .await
    .expect("query repriced api keys row");

    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read api keys cost")
            .expect("api keys cost should exist")
            - 0.02)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cost_estimated")
            .expect("read api keys cost_estimated"),
        Some(1)
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read api keys price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@requested-tier")
    );

    let payload: String = row.try_get("payload").expect("read api keys payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode api keys payload JSON");
    assert_eq!(payload_json["serviceTier"], "default");
    assert_eq!(payload_json["billingServiceTier"], "priority");
    assert_eq!(payload_json["upstreamAccountKind"], "api_key_codex");
    assert_eq!(
        payload_json["upstreamBaseUrlHost"],
        "api-keys.vendor.invalid"
    );
}

#[tokio::test]
async fn backfill_proxy_missing_costs_reprices_failed_api_keys_requested_tier_rows() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let created_at = "2026-01-01T00:00:00Z".to_string();
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, upstream_base_url, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(2750_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("API Keys Pool")
    .bind("https://api-keys.vendor.invalid/")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert failed api keys upstream account");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-failed-api-keys-requested-tier-cost-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":2750,"upstreamAccountName":"API Keys Pool","routeMode":"pool"}"#)
    .bind(r#"{"type":"response.failed"}"#)
    .execute(&pool)
    .await
    .expect("insert failed api keys proxy cost row");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("failed api keys requested-tier cost backfill should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_unpriced_model, 0);

    let row = sqlx::query(
        "SELECT status, cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-failed-api-keys-requested-tier-cost-backfill")
    .fetch_one(&pool)
    .await
    .expect("query repriced failed api keys row");

    assert_eq!(
        row.try_get::<String, _>("status")
            .expect("read failed status"),
        "failed"
    );
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read failed api keys cost")
            .expect("failed api keys cost should exist")
            - 0.02)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cost_estimated")
            .expect("read failed api keys cost_estimated"),
        Some(1)
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read failed api keys price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@requested-tier")
    );

    let payload: String = row
        .try_get("payload")
        .expect("read failed api keys payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode failed api keys payload JSON");
    assert_eq!(payload_json["serviceTier"], "default");
    assert_eq!(payload_json["billingServiceTier"], "priority");
    assert_eq!(payload_json["upstreamAccountKind"], "api_key_codex");
    assert_eq!(
        payload_json["upstreamBaseUrlHost"],
        "api-keys.vendor.invalid"
    );
}

#[tokio::test]
async fn backfill_proxy_missing_costs_prefers_payload_account_kind_snapshots_over_live_account_rows()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, upstream_base_url, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(2568_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("API Keys Pool")
    .bind("https://api-keys.vendor.invalid/")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert api keys upstream account");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-api-keys-requested-tier-snapshot")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":2568,"upstreamAccountName":"API Keys Pool","upstreamAccountKind":"api_key_codex","upstreamBaseUrlHost":"api-keys.vendor.invalid","routeMode":"pool"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert api keys proxy snapshot row");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary_first = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("first api keys snapshot backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET kind = ?1,
            upstream_base_url = ?2,
            updated_at = ?3
        WHERE id = ?4
        "#,
    )
    .bind("oauth_codex")
    .bind("https://oauth.vendor.invalid/")
    .bind(format_utc_iso(Utc::now()))
    .bind(2568_i64)
    .execute(&pool)
    .await
    .expect("mutate live api keys account");

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET cost = ?1,
            cost_estimated = ?2,
            price_version = ?3,
            payload = json_set(payload, '$.billingServiceTier', NULL)
        WHERE invoke_id = ?4
        "#,
    )
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind("proxy-api-keys-requested-tier-snapshot")
    .execute(&pool)
    .await
    .expect("regress api keys snapshot row");

    let summary_second = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("second api keys snapshot backfill should still use payload snapshot");
    assert_eq!(summary_second.scanned, 1);
    assert_eq!(summary_second.updated, 1);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-api-keys-requested-tier-snapshot")
    .fetch_one(&pool)
    .await
    .expect("query api keys snapshot row");
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read api keys snapshot cost")
            .expect("api keys snapshot cost should exist")
            - 0.02)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cost_estimated")
            .expect("read api keys snapshot cost_estimated"),
        Some(1)
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read api keys snapshot price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@requested-tier")
    );

    let payload: String = row
        .try_get("payload")
        .expect("read api keys snapshot payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode api keys snapshot payload JSON");
    assert_eq!(payload_json["billingServiceTier"], "priority");
    assert_eq!(payload_json["upstreamAccountKind"], "api_key_codex");
    assert_eq!(
        payload_json["upstreamBaseUrlHost"],
        "api-keys.vendor.invalid"
    );
}

#[tokio::test]
async fn backfill_proxy_missing_costs_falls_back_to_safe_live_api_key_account_kind_when_snapshot_missing()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let created_at = "2026-01-01T00:00:00Z".to_string();
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, upstream_base_url, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(6144_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("API Keys Safe Live")
    .bind("https://api-keys.safe.invalid/v1")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert safe live api keys upstream account");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-safe-live-api-keys")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":6144,"upstreamAccountName":"API Keys Safe Live","routeMode":"pool"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert safe live api keys invocation");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("safe live api keys rows should use the requested-tier strategy");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-safe-live-api-keys")
    .fetch_one(&pool)
    .await
    .expect("query safe live api keys row");
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read safe live api keys cost")
            .expect("safe live api keys cost should exist")
            - 0.02)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read safe live api keys price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@requested-tier")
    );

    let payload: String = row
        .try_get("payload")
        .expect("read safe live api keys payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode safe live api keys payload JSON");
    assert_eq!(payload_json["billingServiceTier"], "priority");
    assert_eq!(payload_json["upstreamAccountKind"], "api_key_codex");
    assert_eq!(payload_json["upstreamBaseUrlHost"], "api-keys.safe.invalid");
}

#[tokio::test]
async fn backfill_proxy_missing_costs_keeps_response_tier_when_live_account_created_after_invocation()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, upstream_base_url, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(5120_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Late API Keys Account")
    .bind("https://late-api-keys.invalid/")
    .bind("active")
    .bind(1_i64)
    .bind("2026-03-01T00:00:00Z")
    .bind("2026-03-01T00:00:00Z")
    .execute(&pool)
    .await
    .expect("insert late api keys upstream account");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-late-live-api-keys")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":5120,"upstreamAccountName":"Late API Keys Account","routeMode":"pool"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert late live api keys invocation");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("late live accounts should fall back to the response-tier strategy");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-late-live-api-keys")
    .fetch_one(&pool)
    .await
    .expect("query late live api keys row");
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read late live api keys cost")
            .expect("late live api keys cost should exist")
            - 0.01)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read late live api keys price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@response-tier")
    );

    let payload: String = row
        .try_get("payload")
        .expect("read late live api keys payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode late live api keys payload JSON");
    assert_eq!(payload_json["billingServiceTier"], "default");
    assert_eq!(payload_json.get("upstreamAccountKind"), Some(&Value::Null));
    assert_eq!(payload_json.get("upstreamBaseUrlHost"), Some(&Value::Null));
}

#[tokio::test]
async fn backfill_proxy_missing_costs_keeps_non_api_keys_rows_on_response_tier_strategy() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-oauth-response-tier")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountKind":"oauth_codex","routeMode":"pool"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert non-api-keys response-tier row");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("non-api-keys rows should stay on the response-tier strategy");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let row = sqlx::query(
        "SELECT cost, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-oauth-response-tier")
    .fetch_one(&pool)
    .await
    .expect("query non-api-keys response-tier row");
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read non-api-keys cost")
            .expect("non-api-keys cost should exist")
            - 0.01)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read non-api-keys price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@response-tier")
    );

    let payload: String = row.try_get("payload").expect("read non-api-keys payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode non-api-keys payload JSON");
    assert_eq!(payload_json["billingServiceTier"], "default");
}

#[tokio::test]
async fn backfill_proxy_missing_costs_skips_rows_already_settled_with_requested_tier_strategy() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-api-keys-requested-tier-settled")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.02_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23@requested-tier")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"flex","billingServiceTier":"priority","upstreamAccountKind":"api_key_codex","upstreamBaseUrlHost":"api-keys.vendor.invalid","routeMode":"pool"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert settled requested-tier invocation");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("settled requested-tier rows should remain idempotent");
    assert_eq!(summary.scanned, 0);
    assert_eq!(summary.updated, 0);
}

#[tokio::test]
async fn backfill_proxy_missing_costs_skips_missing_model_or_usage_and_retries_unpriced_rows() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-missing-model",
        None,
        Some(1_000),
        Some(500),
    )
    .await;
    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-unpriced-model",
        Some("unknown-model"),
        Some(1_000),
        Some(500),
    )
    .await;
    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-missing-usage",
        Some("gpt-5.2"),
        None,
        None,
    )
    .await;

    let catalog = PricingCatalog {
        version: "unit-cost-backfill".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("cost backfill should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_unpriced_model, 1);
    let expected_attempt_version = pricing_backfill_attempt_version(&catalog);

    let unknown_row = sqlx::query(
        "SELECT cost, cost_estimated, price_version FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-cost-backfill-unpriced-model")
    .fetch_one(&pool)
    .await
    .expect("query unpriced model row");
    assert_eq!(
        unknown_row
            .try_get::<Option<f64>, _>("cost")
            .expect("read unknown cost"),
        None
    );
    assert_eq!(
        unknown_row
            .try_get::<Option<i64>, _>("cost_estimated")
            .expect("read unknown cost_estimated"),
        Some(0)
    );
    assert_eq!(
        unknown_row
            .try_get::<Option<String>, _>("price_version")
            .expect("read unknown price_version")
            .as_deref(),
        Some(expected_attempt_version.as_str())
    );

    let summary_same_version = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("same-version cost backfill should skip attempted unpriced rows");
    assert_eq!(summary_same_version.scanned, 0);
    assert_eq!(summary_same_version.updated, 0);

    let updated_catalog_same_version = PricingCatalog {
        version: catalog.version.clone(),
        models: HashMap::from([
            (
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 2.0,
                    output_per_1m: 3.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
            (
                "unknown-model".to_string(),
                ModelPricing {
                    input_per_1m: 4.0,
                    output_per_1m: 6.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
        ]),
    };
    let summary_same_version_after_pricing_update =
        backfill_proxy_missing_costs(&pool, &updated_catalog_same_version)
            .await
            .expect("same-version pricing update should retry previously unpriced rows");
    assert_eq!(summary_same_version_after_pricing_update.scanned, 1);
    assert_eq!(summary_same_version_after_pricing_update.updated, 1);
    assert_eq!(
        summary_same_version_after_pricing_update.skipped_unpriced_model,
        0
    );

    let unknown_cost_after_update: Option<f64> =
        sqlx::query_scalar("SELECT cost FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-cost-backfill-unpriced-model")
            .fetch_one(&pool)
            .await
            .expect("query unknown model cost after pricing update");
    let expected_unknown_cost = ((1_000.0 * 4.0) + (500.0 * 6.0)) / 1_000_000.0;
    assert!(
        (unknown_cost_after_update.expect("unknown cost should be backfilled")
            - expected_unknown_cost)
            .abs()
            < 1e-12
    );
}

#[test]
fn is_sqlite_lock_error_detects_structured_sqlite_codes() {
    let busy_code_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "5",
        },
    )));
    assert!(is_sqlite_lock_error(&busy_code_error));

    let sqlite_busy_name_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "SQLITE_BUSY",
        },
    )));
    assert!(is_sqlite_lock_error(&sqlite_busy_name_error));

    let non_lock_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "SQLITE_CONSTRAINT",
        },
    )));
    assert!(!is_sqlite_lock_error(&non_lock_error));
}

#[tokio::test]
async fn run_best_effort_retention_pragma_tolerates_sqlite_lock_errors() {
    let err = run_best_effort_retention_pragma(
        &SqlitePool::connect_lazy("sqlite::memory:").expect("construct lazy sqlite pool"),
        "SELECT 1",
        "retention wal checkpoint",
    )
    .await;
    assert!(err.is_ok());

    let locked = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "database table is locked",
            code: "SQLITE_LOCKED",
        },
    )));
    assert!(is_sqlite_lock_error(&locked));
}

#[tokio::test]
async fn build_sqlite_connect_options_enforces_wal_and_busy_timeout_defaults() {
    let temp_dir = make_temp_test_dir("sqlite-connect-options");
    let db_path = temp_dir.join("options.db");
    let db_url = sqlite_url_for_path(&db_path);

    let options = build_sqlite_connect_options(
        &db_url,
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .expect("build sqlite connect options");
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .expect("connect sqlite with options");

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode;")
        .fetch_one(&mut conn)
        .await
        .expect("read pragma journal_mode");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    let busy_timeout_ms: i64 = sqlx::query_scalar("PRAGMA busy_timeout;")
        .fetch_one(&mut conn)
        .await
        .expect("read pragma busy_timeout");
    assert_eq!(
        busy_timeout_ms,
        (DEFAULT_SQLITE_BUSY_TIMEOUT_SECS * 1_000) as i64
    );

    conn.close().await.expect("close sqlite connection");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_backfill_with_retry_succeeds_after_lock_release() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-success");
    let db_path = temp_dir.join("lock-success.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload(&response_path);
    insert_proxy_backfill_row(&pool, "proxy-lock-retry-success", &response_path).await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let backfill_task =
        tokio::spawn(async move { run_backfill_with_retry(&pool_for_task, None).await });

    tokio::time::sleep(Duration::from_millis(400)).await;
    sqlx::query("COMMIT")
        .execute(&mut lock_conn)
        .await
        .expect("release sqlite write lock");

    let summary = backfill_task
        .await
        .expect("join backfill task")
        .expect("backfill should succeed after retry");
    assert!(
        started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "expected retry delay to be applied"
    );
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-lock-retry-success")
            .fetch_one(&pool)
            .await
            .expect("query backfilled row");
    assert_eq!(total_tokens, Some(110));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_backfill_with_retry_fails_when_lock_persists() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-fail");
    let db_path = temp_dir.join("lock-fail.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload(&response_path);
    insert_proxy_backfill_row(&pool, "proxy-lock-retry-fail", &response_path).await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let backfill_task =
        tokio::spawn(async move { run_backfill_with_retry(&pool_for_task, None).await });
    let err = backfill_task
        .await
        .expect("join backfill task")
        .expect_err("backfill should fail after lock retry exhaustion");
    assert!(
        started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "expected retry delay before final failure"
    );
    assert!(
        err.to_string().contains("failed after 2/2 attempt(s)"),
        "expected retry exhaustion context in error: {err:?}"
    );
    assert!(is_sqlite_lock_error(&err));

    let total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-lock-retry-fail")
            .fetch_one(&pool)
            .await
            .expect("query locked row");
    assert_eq!(total_tokens, None);

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("rollback lock holder");
    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_backfill_with_retry_does_not_retry_non_lock_errors() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-non-lock");
    let db_path = temp_dir.join("non-lock.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");

    // Intentionally skip schema initialization to force a deterministic non-lock error.
    let started = Instant::now();
    let err = run_backfill_with_retry(&pool, None)
        .await
        .expect_err("backfill should fail immediately on non-lock errors");
    assert!(
        started.elapsed() < Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "non-lock errors should not wait for retry delay"
    );
    assert!(
        err.to_string().contains("failed after 1/2 attempt(s)"),
        "expected single-attempt context in error: {err:?}"
    );
    assert!(!is_sqlite_lock_error(&err));
    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("no such table")
    }));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_cost_backfill_with_retry_succeeds_after_lock_release() {
    let temp_dir = make_temp_test_dir("proxy-cost-backfill-retry-success");
    let db_path = temp_dir.join("lock-success.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-lock-retry-success",
        Some("gpt-5.2-2025-12-11"),
        Some(2_000),
        Some(1_000),
    )
    .await;
    let catalog = PricingCatalog {
        version: "unit-cost-retry".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let catalog_for_task = catalog.clone();
    let backfill_task = tokio::spawn(async move {
        run_cost_backfill_with_retry(&pool_for_task, &catalog_for_task).await
    });

    tokio::time::sleep(Duration::from_millis(400)).await;
    sqlx::query("COMMIT")
        .execute(&mut lock_conn)
        .await
        .expect("release sqlite write lock");

    let summary = backfill_task
        .await
        .expect("join cost backfill task")
        .expect("cost backfill should succeed after retry");
    assert!(
        started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "expected retry delay to be applied"
    );
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let cost: Option<f64> =
        sqlx::query_scalar("SELECT cost FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-cost-lock-retry-success")
            .fetch_one(&pool)
            .await
            .expect("query backfilled cost row");
    assert!(cost.is_some());

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_cost_backfill_with_retry_does_not_retry_non_lock_errors() {
    let temp_dir = make_temp_test_dir("proxy-cost-backfill-retry-non-lock");
    let db_path = temp_dir.join("non-lock.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    let catalog = PricingCatalog {
        version: "unit-cost-retry".to_string(),
        models: HashMap::new(),
    };

    // Intentionally skip schema initialization to force a deterministic non-lock error.
    let started = Instant::now();
    let err = run_cost_backfill_with_retry(&pool, &catalog)
        .await
        .expect_err("cost backfill should fail immediately on non-lock errors");
    assert!(
        started.elapsed() < Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "non-lock errors should not wait for retry delay"
    );
    assert!(
        err.to_string().contains("failed after 1/2 attempt(s)"),
        "expected single-attempt context in error: {err:?}"
    );
    assert!(!is_sqlite_lock_error(&err));
    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("no such table")
    }));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn quota_latest_returns_degraded_when_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let config = test_config();
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config))),
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    });

    let Json(snapshot) = latest_quota_snapshot(State(state))
        .await
        .expect("route should succeed");

    assert!(!snapshot.is_active);
    assert_eq!(snapshot.total_requests, 0);
    assert_eq!(snapshot.total_cost, 0.0);
}

#[tokio::test]
async fn quota_latest_returns_seeded_historical_snapshot() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let captured_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &captured_at).await;

    let Json(snapshot) = latest_quota_snapshot(State(state))
        .await
        .expect("route should return seeded quota snapshot");

    assert_eq!(snapshot.captured_at, captured_at);
    let snapshot_json = serde_json::to_value(&snapshot).expect("serialize quota snapshot");
    assert!(
        snapshot_json["capturedAt"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z')),
        "serialized quota snapshot should emit UTC ISO timestamps"
    );
    assert!(snapshot.is_active);
    assert_eq!(snapshot.total_requests, 9);
    assert_f64_close(snapshot.total_cost, 10.0);
}

async fn insert_timeseries_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    status: &str,
    t_upstream_ttfb_ms: Option<f64>,
) {
    insert_timeseries_invocation_with_stages(
        pool,
        invoke_id,
        occurred_at,
        status,
        None,
        None,
        None,
        t_upstream_ttfb_ms,
    )
    .await;
}

async fn insert_timeseries_invocation_with_stages(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    status: &str,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(status)
    .bind(10_i64)
    .bind(0.01_f64)
    .bind(t_req_read_ms)
    .bind(t_req_parse_ms)
    .bind(t_upstream_connect_ms)
    .bind(t_upstream_ttfb_ms)
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert timeseries invocation");
}

async fn insert_parallel_work_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    prompt_cache_key: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(invoke_id)
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10_i64)
    .bind(0.01_f64)
    .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert parallel-work invocation");
}

async fn insert_invocation_rollup(
    pool: &SqlitePool,
    stats_date: NaiveDate,
    source: &str,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
) {
    insert_invocation_rollup_with_latency_samples(
        pool,
        stats_date,
        source,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        total_cost,
        &[],
        &[],
    )
    .await;
}

fn encode_histogram_from_samples(samples: &[f64]) -> String {
    let mut histogram = empty_approx_histogram();
    for sample in samples {
        add_approx_histogram_sample(&mut histogram, *sample);
    }
    encode_approx_histogram(&histogram).expect("encode approximate histogram from samples")
}

fn sum_f64_samples(samples: &[f64]) -> f64 {
    samples.iter().copied().sum::<f64>()
}

fn max_f64_sample(samples: &[f64]) -> f64 {
    samples
        .iter()
        .copied()
        .fold(0.0_f64, |current, value| current.max(value))
}

async fn insert_invocation_rollup_with_latency_samples(
    pool: &SqlitePool,
    stats_date: NaiveDate,
    source: &str,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    first_byte_samples: &[f64],
    first_response_byte_total_samples: &[f64],
) {
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_daily (
            stats_date,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
        "#,
    )
    .bind(stats_date.to_string())
    .bind(source)
    .bind(total_count)
    .bind(success_count)
    .bind(failure_count)
    .bind(total_tokens)
    .bind(total_cost)
    .execute(pool)
    .await
    .expect("insert invocation rollup");

    let bucket_start_epoch = local_naive_to_utc(
        stats_date
            .and_hms_opt(0, 0, 0)
            .expect("stats_date midnight should be valid"),
        Shanghai,
    )
    .timestamp();
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(source)
    .bind(total_count)
    .bind(success_count)
    .bind(failure_count)
    .bind(total_tokens)
    .bind(total_cost)
    .bind(first_byte_samples.len() as i64)
    .bind(sum_f64_samples(first_byte_samples))
    .bind(max_f64_sample(first_byte_samples))
    .bind(encode_histogram_from_samples(first_byte_samples))
    .bind(first_response_byte_total_samples.len() as i64)
    .bind(sum_f64_samples(first_response_byte_total_samples))
    .bind(max_f64_sample(first_response_byte_total_samples))
    .bind(encode_histogram_from_samples(
        first_response_byte_total_samples,
    ))
    .execute(pool)
    .await
    .expect("insert invocation hourly rollup");
}

async fn insert_invocation_hourly_rollup_bucket(
    pool: &SqlitePool,
    bucket_start: DateTime<Utc>,
    source: &str,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
) {
    insert_invocation_hourly_rollup_bucket_with_latency_samples(
        pool,
        bucket_start,
        source,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        total_cost,
        &[],
        &[],
    )
    .await;
}

async fn insert_invocation_hourly_rollup_bucket_with_latency_samples(
    pool: &SqlitePool,
    bucket_start: DateTime<Utc>,
    source: &str,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    first_byte_samples: &[f64],
    first_response_byte_total_samples: &[f64],
) {
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
        "#,
    )
    .bind(bucket_start.timestamp())
    .bind(source)
    .bind(total_count)
    .bind(success_count)
    .bind(failure_count)
    .bind(total_tokens)
    .bind(total_cost)
    .bind(first_byte_samples.len() as i64)
    .bind(sum_f64_samples(first_byte_samples))
    .bind(max_f64_sample(first_byte_samples))
    .bind(encode_histogram_from_samples(first_byte_samples))
    .bind(first_response_byte_total_samples.len() as i64)
    .bind(sum_f64_samples(first_response_byte_total_samples))
    .bind(max_f64_sample(first_response_byte_total_samples))
    .bind(encode_histogram_from_samples(
        first_response_byte_total_samples,
    ))
    .execute(pool)
    .await
    .expect("insert invocation hourly rollup bucket");
}
