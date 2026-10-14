use super::*;

#[tokio::test]
async fn retry_attempt_retains_completed_raw_capture_after_runtime_metadata_rejection() {
    let fixture = std::env::temp_dir().join(format!("retry-raw-metadata-{}", nanoid::nanoid!()));
    fs::create_dir_all(&fixture).expect("create owned raw metadata fixture");
    let mut config = test_config();
    config.database_path = fixture.join("business.sqlite");
    config.proxy_raw_dir = fixture.join("raw");
    config.proxy_raw_compression = RawCompressionCodec::None;
    config.proxy_raw_max_bytes = Some(5);
    let mut state = test_state_from_config(config, true).await;
    let mutable = Arc::get_mut(&mut state).expect("fixture has one state owner");
    mutable.sqlite_batch_writer = SqliteBatchWriter::spawn(
        mutable.pool.clone(),
        mutable.shutdown.clone(),
        mutable.prompt_cache_conversation_cache.clone(),
        mutable.pricing_catalog.clone(),
        &mutable.config.database_path,
    )
    .expect("production file journal");
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    let account = insert_test_pool_api_key_account(&state, "Retry Raw", "fixture-token").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "retry-raw-metadata".to_string(),
        occurred_at: shanghai_now_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: None,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: None,
    };
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let admissions_before = coordinator
        .snapshot()
        .await
        .write_admission
        .interactive_proxy
        .admission_count;
    let attempt = begin_pool_upstream_request_attempt_in_memory(
        &state,
        &trace,
        None,
        None,
        None,
        None,
        None,
        None,
        account,
        "https://example.invalid/",
        1,
        1,
        0,
        &trace.occurred_at,
    )
    .await
    .expect("begin runtime attempt");
    let rejections_before = state
        .proxy_runtime_attempts
        .health_snapshot()
        .rejected_records;
    state
        .proxy_runtime_attempts
        .reject_next_response_capture_for_test();
    persist_pool_attempt_response_capture(
        state.clone(),
        attempt.clone(),
        Bytes::from_static(b"synthetic retry response"),
        true,
        Some("identity".to_string()),
    )
    .await;
    assert_eq!(
        state
            .proxy_runtime_attempts
            .health_snapshot()
            .rejected_records,
        rejections_before + 1,
        "exercise the bounded metadata rejection path"
    );
    finalize_pool_upstream_request_attempt_in_memory(
        &state,
        &attempt,
        &trace.occurred_at,
        "http_failure",
        Some(StatusCode::BAD_GATEWAY),
        None,
        Some("fixture_failure"),
        Some("synthetic retry failure"),
        None,
        Some(1.0),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("finalize the actual pending attempt");
    persist_and_broadcast_proxy_capture_terminal_record(
        &state,
        test_proxy_capture_record(&trace.invoke_id, &trace.occurred_at),
    )
    .await
    .expect("confirm the final snapshot through the journal");
    assert_eq!(
        coordinator
            .snapshot()
            .await
            .write_admission
            .interactive_proxy
            .admission_count,
        admissions_before,
        "runtime metadata rejection cannot fall back to SQLite admission"
    );
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("materialize final snapshot");
    let (path, codec, size, truncated, reason, encoding) = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<String>,
            Option<i64>,
            bool,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated, \
         response_raw_truncated_reason, response_content_encoding \
         FROM pool_upstream_request_attempts WHERE attempt_public_id = ?1",
    )
    .bind(attempt.attempt_public_id.as_deref())
    .fetch_one(&state.pool)
    .await
    .expect("read the durable attempt fact");
    let path = path.expect("completed raw file must be referenced by the terminal attempt");
    assert_eq!(fs::read(path).expect("read retained raw file"), b"synth");
    assert_eq!(codec.as_deref(), Some("identity"));
    assert_eq!(size, Some(24));
    assert!(truncated);
    assert_eq!(reason.as_deref(), Some("max_bytes_exceeded"));
    assert_eq!(encoding.as_deref(), Some("identity"));
    state.shutdown.cancel();
    state.sqlite_batch_writer.shutdown_and_drain().await;
    drop(state);
    fs::remove_dir_all(fixture).expect("remove exact owned fixture");
}
