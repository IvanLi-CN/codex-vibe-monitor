#[tokio::test]
pub(crate) async fn ensure_schema_adds_upstream_account_pressure_hot_path_indexes() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    for index_name in [
        "idx_pool_upstream_accounts_maintenance_due",
        "idx_pool_upstream_account_events_time",
        "idx_pool_oauth_login_sessions_status_expires",
        "idx_pool_limit_samples_account_captured_desc",
    ] {
        let exists: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM sqlite_master
            WHERE type = 'index' AND name = ?1
            "#,
        )
        .bind(index_name)
        .fetch_one(&pool)
        .await
        .expect("query sqlite indexes");
        assert_eq!(exists, 1, "missing index {index_name}");
    }
}

#[tokio::test]
pub(crate) async fn backfill_invocation_service_tiers_revisits_inline_proxy_auto_tiers_without_raw_files()
 {
    let pool = test_current_schema_pool().await;

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
pub(crate) async fn backfill_invocation_service_tiers_revisits_inline_proxy_non_auto_stream_tiers()
{
    let pool = test_current_schema_pool().await;

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
pub(crate) async fn backfill_invocation_service_tiers_tracks_skip_counters() {
    let pool = test_current_schema_pool().await;

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

fn write_usage_backfill_response() -> (PathBuf, PathBuf) {
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
    (temp_dir, response_path)
}

async fn seed_usage_backfill_rows(pool: &SqlitePool, response_path: &Path, row_count: usize) {
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
        .execute(pool)
        .await
        .expect("insert proxy row");
    }
}

async fn assert_usage_backfill_rows(pool: &SqlitePool, row_count: usize) {
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
    .fetch_one(pool)
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
}

#[tokio::test]
pub(crate) async fn backfill_proxy_usage_tokens_updates_missing_tokens_idempotently() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");
    let (temp_dir, response_path) = write_usage_backfill_response();
    let row_count = BACKFILL_BATCH_SIZE as usize + 5;
    seed_usage_backfill_rows(&pool, &response_path, row_count).await;

    let summary_first = backfill_proxy_usage_tokens(&pool, None)
        .await
        .expect("first backfill should succeed");
    assert_eq!(summary_first.scanned, row_count as u64);
    assert_eq!(summary_first.updated, row_count as u64);
    assert_usage_backfill_rows(&pool, row_count).await;

    let summary_second = backfill_proxy_usage_tokens(&pool, None)
        .await
        .expect("second backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_proxy_usage_tokens_reads_from_fallback_root_for_relative_paths() {
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
pub(crate) async fn backfill_proxy_usage_tokens_respects_snapshot_upper_bound() {
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
pub(crate) async fn backfill_proxy_missing_costs_updates_dated_model_alias_and_is_idempotent() {
    let pool = test_current_schema_pool().await;

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
                cache_read_per_1m: None,
                cache_write_per_1m: None,
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
pub(crate) async fn backfill_proxy_missing_costs_backfills_standard_rows_with_missing_billing_service_tier()
 {
    let pool = test_current_schema_pool().await;
    insert_proxy_cost_row(
        &pool,
        ProxyCostRowSeed {
            invoke_id: "proxy-standard-null-billing-tier",
            status: "success",
            model: "gpt-5.2",
            input_tokens: 1_000,
            output_tokens: 500,
            cost: 0.0035,
            price_version: "unit-cost-backfill",
            payload: r#"{"endpoint":"/v1/responses","serviceTier":"default","billingServiceTier":null}"#,
            raw_response: "{}",
        },
    )
    .await;
    let catalog = unit_cost_backfill_catalog();

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
pub(crate) async fn backfill_proxy_missing_costs_rewrites_stale_standard_billing_service_tier() {
    let pool = test_current_schema_pool().await;

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
                cache_read_per_1m: None,
                cache_write_per_1m: None,
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
pub(crate) async fn backfill_proxy_missing_costs_reprices_api_keys_requested_tier_rows() {
    let pool = test_current_schema_pool().await;
    insert_api_key_upstream_account(
        &pool,
        UpstreamAccountSeed {
            id: 2568,
            display_name: "API Keys Pool",
            upstream_base_url: "https://api-keys.vendor.invalid/",
            created_at: "2026-01-01T00:00:00Z",
        },
    )
    .await;
    insert_proxy_cost_row(
        &pool,
        ProxyCostRowSeed {
            invoke_id: "proxy-api-keys-requested-tier-cost-backfill",
            status: "success",
            model: "gpt-5.4",
            input_tokens: 1_000,
            output_tokens: 500,
            cost: 0.01,
            price_version: "openai-standard-2026-02-23",
            payload: r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":2568,"upstreamAccountName":"API Keys Pool","routeMode":"pool"}"#,
            raw_response: "{}",
        },
    )
    .await;
    let catalog = openai_standard_backfill_catalog();

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
pub(crate) async fn backfill_proxy_missing_costs_reprices_failed_api_keys_requested_tier_rows() {
    let pool = test_current_schema_pool().await;
    insert_api_key_upstream_account(
        &pool,
        UpstreamAccountSeed {
            id: 2750,
            display_name: "API Keys Pool",
            upstream_base_url: "https://api-keys.vendor.invalid/",
            created_at: "2026-01-01T00:00:00Z",
        },
    )
    .await;
    insert_proxy_cost_row(
        &pool,
        ProxyCostRowSeed {
            invoke_id: "proxy-failed-api-keys-requested-tier-cost-backfill",
            status: "failed",
            model: "gpt-5.4",
            input_tokens: 1_000,
            output_tokens: 500,
            cost: 0.01,
            price_version: "openai-standard-2026-02-23",
            payload: r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":2750,"upstreamAccountName":"API Keys Pool","routeMode":"pool"}"#,
            raw_response: r#"{"type":"response.failed"}"#,
        },
    )
    .await;
    let catalog = openai_standard_backfill_catalog();

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

use super::*;
