#[tokio::test]
pub(crate) async fn refresh_forward_proxy_subscriptions_triggers_bootstrap_probe_for_added_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes before subscription refresh");
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let success_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;

    refresh_forward_proxy_subscriptions(state.clone(), true, None)
        .await
        .expect("refresh subscriptions should succeed");
    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let success_count =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;
    assert!(
        success_count > success_count_before,
        "expected at least one successful bootstrap probe attempt from subscription refresh"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn refresh_forward_proxy_subscriptions_skips_probe_for_known_subscription_keys() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes before subscription refresh");

    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let known_keys = HashSet::from([proxy_key.clone()]);
    refresh_forward_proxy_subscriptions(state.clone(), true, Some(known_keys))
        .await
        .expect("refresh subscriptions should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let probe_count_after = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(
        probe_count_after, probe_count_before,
        "known subscription keys should suppress startup-style reprobe"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn forward_proxy_settings_bootstrap_probe_failure_penalizes_runtime_weight() {
    let (proxy_url, proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let failure_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(false)).await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec![proxy_url],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");

    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let failure_count =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(false)).await;
    assert!(
        failure_count > failure_count_before,
        "expected at least one failed bootstrap probe attempt"
    );

    let runtime_weight = read_forward_proxy_runtime_weight(&state.pool, &proxy_key)
        .await
        .expect("runtime weight should exist");
    assert!(
        runtime_weight < 1.0,
        "expected failed bootstrap probe to penalize runtime weight; got {runtime_weight}"
    );

    proxy_handle.abort();
}

#[test]
pub(crate) fn forward_proxy_manager_keeps_one_positive_weight() {
    let mut manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );

    for runtime in manager.runtime.values_mut() {
        runtime.weight = -5.0;
    }
    manager.ensure_non_zero_weight();

    assert!(manager.runtime.values().any(|entry| entry.weight > 0.0));
}

#[test]
pub(crate) fn forward_proxy_algo_from_str_supports_v1_and_v2() {
    assert_eq!(
        ForwardProxyAlgo::from_str("v1").expect("v1 should parse"),
        ForwardProxyAlgo::V1
    );
    assert_eq!(
        ForwardProxyAlgo::from_str("V2").expect("v2 should parse"),
        ForwardProxyAlgo::V2
    );
    assert!(ForwardProxyAlgo::from_str("unexpected").is_err());
}

#[test]
pub(crate) fn forward_proxy_algo_config_defaults_to_latest_v2() {
    let algo = resolve_forward_proxy_algo_config(None, None).expect("default algo should resolve");
    assert_eq!(algo, ForwardProxyAlgo::V2);
}

#[test]
pub(crate) fn forward_proxy_algo_config_accepts_primary_env() {
    let algo =
        resolve_forward_proxy_algo_config(Some("v2"), None).expect("primary env should resolve");
    assert_eq!(algo, ForwardProxyAlgo::V2);
}

#[test]
pub(crate) fn forward_proxy_algo_config_rejects_legacy_env() {
    let err =
        resolve_forward_proxy_algo_config(None, Some("v1")).expect_err("legacy env should fail");
    assert_eq!(
        err.to_string(),
        "XY_FORWARD_PROXY_ALGO is not supported; rename it to FORWARD_PROXY_ALGO"
    );
}

#[test]
pub(crate) fn forward_proxy_algo_config_rejects_when_both_env_vars_are_set() {
    let err = resolve_forward_proxy_algo_config(Some("v2"), Some("v1"))
        .expect_err("legacy env should still win as a hard failure");
    assert_eq!(
        err.to_string(),
        "XY_FORWARD_PROXY_ALGO is not supported; rename it to FORWARD_PROXY_ALGO"
    );
}

#[test]
pub(crate) fn forward_proxy_manager_v2_keeps_two_positive_weights() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    for runtime in manager.runtime.values_mut() {
        runtime.weight = -5.0;
    }
    manager.ensure_non_zero_weight();

    let positive_count = manager
        .endpoints
        .iter()
        .filter_map(|endpoint| manager.runtime.get(&endpoint.key))
        .filter(|entry| entry.weight > 0.0)
        .count();
    assert_eq!(positive_count, 1);
}

#[tokio::test]
pub(crate) async fn async_streaming_raw_payload_writer_queues_when_global_writer_pool_is_saturated()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let mut permits = Vec::new();
    while let Ok(permit) = state.proxy_raw_async_semaphore.clone().try_acquire_owned() {
        permits.push(permit);
    }
    assert!(
        !permits.is_empty(),
        "expected raw payload async semaphore to expose permits"
    );
    assert_eq!(state.proxy_raw_async_semaphore.available_permits(), 0);

    let mut writer = AsyncStreamingRawPayloadWriter::new(
        state.as_ref(),
        "invoke-backpressure",
        "response",
        true,
        None,
    );
    writer.append(b"hello");
    let spool_dir = state.config.resolved_proxy_raw_dir().join(".spool");
    assert!(
        fs::read_dir(&spool_dir)
            .expect("overflow spool directory")
            .flatten()
            .any(
                |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("frames")
            ),
        "saturated writer should persist overflow bytes before capacity is released"
    );
    drop(permits);
    let meta = writer.finish().await;

    assert_eq!(meta.size_bytes, 5);
    assert!(!meta.truncated);
    let path = PathBuf::from(meta.path.expect("queued raw capture path"));
    assert_eq!(
        read_proxy_raw_bytes(path.to_string_lossy().as_ref(), None).unwrap(),
        b"hello"
    );
    let _ = fs::remove_file(path);
}

#[tokio::test]
pub(crate) async fn raw_overflow_spool_allows_concurrent_captures_without_preallocating_global_capacity()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut permits = Vec::new();
    while let Ok(permit) = state.proxy_raw_async_semaphore.clone().try_acquire_owned() {
        permits.push(permit);
    }

    let mut first = AsyncStreamingRawPayloadWriter::new(
        state.as_ref(),
        "invoke-concurrent-spool-first",
        "response",
        true,
        None,
    );
    let mut second = AsyncStreamingRawPayloadWriter::new(
        state.as_ref(),
        "invoke-concurrent-spool-second",
        "response",
        true,
        None,
    );
    first.append(b"first");
    second.append(b"second");

    let spool_dir = state.config.resolved_proxy_raw_dir().join(".spool");
    let segment_count = fs::read_dir(&spool_dir)
        .expect("overflow spool directory")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("frames"))
        .count();
    assert_eq!(
        segment_count, 2,
        "small concurrent captures should each receive a durable spool segment"
    );

    drop(permits);
    let first_meta = first.finish().await;
    let second_meta = second.finish().await;
    for (meta, expected) in [
        (first_meta, b"first".as_slice()),
        (second_meta, b"second".as_slice()),
    ] {
        assert!(!meta.truncated);
        let path = PathBuf::from(meta.path.expect("concurrent spool raw capture path"));
        assert_eq!(
            read_proxy_raw_bytes(path.to_string_lossy().as_ref(), None).expect("read raw"),
            expected
        );
        let _ = fs::remove_file(path);
    }
}

#[tokio::test]
pub(crate) async fn raw_overflow_spool_recovery_publishes_complete_frames_and_keeps_invalid_files()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut permits = Vec::new();
    while let Ok(permit) = state.proxy_raw_async_semaphore.clone().try_acquire_owned() {
        permits.push(permit);
    }
    let mut writer = AsyncStreamingRawPayloadWriter::new(
        state.as_ref(),
        "invoke-spool-recovery",
        "response",
        true,
        None,
    );
    writer.append(b"recover-me");
    drop(writer);

    let spool_dir = state.config.resolved_proxy_raw_dir().join(".spool");
    fs::create_dir_all(&spool_dir).expect("create spool dir");
    let invalid_path = spool_dir.join("incomplete.frames");
    fs::write(&invalid_path, b"partial").expect("write invalid spool");

    drop(permits);
    recover_raw_overflow_spools(&state.config).await;

    let recovered = state
        .config
        .resolved_proxy_raw_dir()
        .join("invoke-spool-recovery-response.bin.zst");
    assert_eq!(
        read_proxy_raw_bytes(recovered.to_string_lossy().as_ref(), None).expect("recovered raw"),
        b"recover-me"
    );
    assert!(
        invalid_path.exists(),
        "invalid spool must remain for inspection"
    );
    let _ = fs::remove_file(recovered);
    let _ = fs::remove_file(invalid_path);
}

#[tokio::test]
pub(crate) async fn raw_overflow_spool_rotates_segments_and_recovers_the_capture_in_order() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut permits = Vec::new();
    while let Ok(permit) = state.proxy_raw_async_semaphore.clone().try_acquire_owned() {
        permits.push(permit);
    }
    let mut writer = AsyncStreamingRawPayloadWriter::new(
        state.as_ref(),
        "invoke-spool-segments",
        "response",
        true,
        None,
    );
    let first_segment = vec![b'a'; RAW_OVERFLOW_SPOOL_SEGMENT_BYTES as usize];
    writer.append(&first_segment);
    writer.append(b"tail");
    drop(writer);

    let spool_dir = state.config.resolved_proxy_raw_dir().join(".spool");
    let segment_count = fs::read_dir(&spool_dir)
        .expect("read spool directory")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("frames"))
        .count();
    assert_eq!(segment_count, 2, "overflow capture should rotate at 16 MiB");

    drop(permits);
    recover_raw_overflow_spools(&state.config).await;

    let recovered = state
        .config
        .resolved_proxy_raw_dir()
        .join("invoke-spool-segments-response.bin.zst");
    let payload = read_proxy_raw_bytes(recovered.to_string_lossy().as_ref(), None)
        .expect("recover segmented raw payload");
    assert_eq!(payload.len(), first_segment.len() + 4);
    assert_eq!(&payload[..4], b"aaaa");
    assert_eq!(&payload[payload.len() - 4..], b"tail");
    let _ = fs::remove_file(recovered);
}

#[tokio::test]
pub(crate) async fn raw_overflow_spool_recovery_retains_a_capture_when_a_later_segment_is_corrupt()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut permits = Vec::new();
    while let Ok(permit) = state.proxy_raw_async_semaphore.clone().try_acquire_owned() {
        permits.push(permit);
    }
    let mut writer = AsyncStreamingRawPayloadWriter::new(
        state.as_ref(),
        "invoke-corrupt-segments",
        "response",
        true,
        None,
    );
    writer.append(&vec![b'a'; RAW_OVERFLOW_SPOOL_SEGMENT_BYTES as usize]);
    writer.append(b"tail");
    drop(writer);

    let spool_dir = state.config.resolved_proxy_raw_dir().join(".spool");
    let mut segments = fs::read_dir(&spool_dir)
        .expect("read spool directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("frames"))
        .collect::<Vec<_>>();
    segments.sort();
    assert_eq!(
        segments.len(),
        2,
        "overflow capture should rotate at 16 MiB"
    );
    fs::write(&segments[1], b"partial").expect("corrupt later spool segment");

    drop(permits);
    recover_raw_overflow_spools(&state.config).await;

    let recovered = state
        .config
        .resolved_proxy_raw_dir()
        .join("invoke-corrupt-segments-response.bin.zst");
    assert!(
        !recovered.exists(),
        "a capture with a corrupt later segment must not publish its valid prefix"
    );
    assert!(
        segments.iter().all(|path| path.exists()),
        "all segments must remain available for inspection and recovery"
    );
    for segment in segments {
        let _ = fs::remove_file(segment);
    }
}

#[tokio::test]
pub(crate) async fn non_proxy_terminal_transition_marks_timeseries_projection_recovery() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let minute = Utc::now().timestamp().div_euclid(60) * 60;
    sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state) VALUES (?1, 'all', -1, '{}', '[]', '[]', '[]', '[]', 1, 'ready')",
    )
    .bind(minute)
    .execute(&state.pool)
    .await
    .expect("seed all projection coverage");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, payload, raw_response) VALUES ('non-proxy-running', ?1, 'xy', 'running', '{}', '')",
    )
    .bind(format_utc_iso(Utc.timestamp_opt(minute, 0).single().expect("valid minute")))
    .execute(&state.pool)
    .await
    .expect("seed non-proxy in-flight invocation");

    sqlx::query(
        "UPDATE codex_invocations SET status = 'success' WHERE invoke_id = 'non-proxy-running'",
    )
    .execute(&state.pool)
    .await
    .expect("terminalize non-proxy invocation");

    let coverage_state = sqlx::query_scalar::<_, String>(
        "SELECT coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = 'all' AND upstream_account_key = -1",
    )
    .bind(minute)
    .fetch_one(&state.pool)
    .await
    .expect("load all projection coverage");
    assert_eq!(coverage_state, "ready");
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load durable projection recovery marker");
    assert_eq!(recovery_pending, 1);
}

#[tokio::test]
pub(crate) async fn streaming_raw_capture_preserves_precompressed_wire_bytes_without_recompression()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"already-gzipped-wire-body").unwrap();
    let wire_bytes = encoder.finish().unwrap();

    let mut writer = AsyncStreamingRawPayloadWriter::new(
        state.as_ref(),
        "invoke-precompressed-wire",
        "response",
        true,
        Some("gzip"),
    );
    writer.append(&wire_bytes);
    let meta = writer.finish().await;
    let path = PathBuf::from(meta.path.expect("raw wire path"));
    assert!(path.ends_with("invoke-precompressed-wire-response.bin"));
    assert_eq!(fs::read(&path).expect("read stored wire bytes"), wire_bytes);
    let _ = fs::remove_file(path);
}

#[tokio::test]
pub(crate) async fn spawn_raw_payload_file_write_skips_new_request_raw_files_when_disabled() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let meta = spawn_raw_payload_file_write(
        state.as_ref(),
        "invoke-request-body-disabled",
        "request",
        Bytes::from_static(b"{\"request\":true}"),
        false,
    )
    .finish()
    .await;

    assert!(meta.path.is_none());
    assert_eq!(meta.size_bytes, br#"{"request":true}"#.len() as i64);
    assert!(!meta.truncated);
    assert!(meta.truncated_reason.is_none());
}

#[test]
pub(crate) fn legacy_bound_proxy_keys_still_route_to_matching_stable_endpoints() {
    let legacy_proxy_url = "http://127.0.0.1:7890";
    let stable_proxy_key =
        normalize_single_proxy_key(legacy_proxy_url).expect("legacy proxy url should normalize");
    let mut manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![legacy_proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );

    let scope = ForwardProxyRouteScope::from_group_binding(
        Some("东京组"),
        vec![legacy_proxy_url.to_string()],
    );
    let selected = manager
        .select_proxy_for_scope(&scope)
        .expect("legacy bound key should still select proxy");

    assert_eq!(selected.key, stable_proxy_key);
}

#[test]
pub(crate) fn legacy_vless_and_trojan_bound_proxy_keys_route_to_matching_stable_endpoints() {
    let explicit_vless_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?encryption=none&security=none&type=tcp#东京节点";
    let normalized_vless_proxy_url = normalize_share_link_scheme(explicit_vless_proxy_url, "vless")
        .expect("normalize vless url");
    let explicit_legacy_vless_proxy_key = {
        let parsed = Url::parse(&normalized_vless_proxy_url).expect("parse normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let omitted_default_vless_proxy_key = stable_forward_proxy_key(&canonical_share_link_identity(
        &Url::parse("vless://11111111-1111-1111-1111-111111111111@vless.example.com:443#东京节点")
            .expect("parse omitted-default vless url"),
    ));
    let vless_aliases =
        legacy_bound_proxy_key_aliases(&normalized_vless_proxy_url, ForwardProxyProtocol::Vless);
    assert!(vless_aliases.contains(&explicit_legacy_vless_proxy_key));
    assert!(vless_aliases.contains(&omitted_default_vless_proxy_key));
    assert!(
        legacy_bound_proxy_key_aliases(&normalized_vless_proxy_url, ForwardProxyProtocol::Trojan)
            .is_empty()
    );
    let stable_vless_proxy_key =
        normalize_single_proxy_key(explicit_vless_proxy_url).expect("stable vless proxy key");
    assert_ne!(explicit_legacy_vless_proxy_key, stable_vless_proxy_key);
    assert_ne!(omitted_default_vless_proxy_key, stable_vless_proxy_key);

    let mut vless_manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![explicit_vless_proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );
    for endpoint in &mut vless_manager.endpoints {
        endpoint.endpoint_url = Some(
            Url::parse("socks5://127.0.0.1:11080").expect("parse synthesized vless endpoint url"),
        );
    }
    let vless_scope = ForwardProxyRouteScope::from_group_binding(
        Some("东京组"),
        vec![omitted_default_vless_proxy_key.clone()],
    );
    let selected_vless = vless_manager
        .select_proxy_for_scope(&vless_scope)
        .expect("legacy vless bound key should still select proxy");
    assert_eq!(selected_vless.key, stable_vless_proxy_key);

    let explicit_trojan_proxy_url =
        "trojan://password@trojan.example.com:443?security=tls&type=tcp#东京节点";
    let normalized_trojan_proxy_url =
        normalize_share_link_scheme(explicit_trojan_proxy_url, "trojan")
            .expect("normalize trojan url");
    let explicit_legacy_trojan_proxy_key = {
        let parsed = Url::parse(&normalized_trojan_proxy_url).expect("parse normalized trojan url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let omitted_default_trojan_proxy_key =
        stable_forward_proxy_key(&canonical_share_link_identity(
            &Url::parse("trojan://password@trojan.example.com:443#东京节点")
                .expect("parse omitted-default trojan url"),
        ));
    let trojan_aliases =
        legacy_bound_proxy_key_aliases(&normalized_trojan_proxy_url, ForwardProxyProtocol::Trojan);
    assert!(trojan_aliases.contains(&explicit_legacy_trojan_proxy_key));
    assert!(trojan_aliases.contains(&omitted_default_trojan_proxy_key));
    assert!(
        legacy_bound_proxy_key_aliases(&normalized_trojan_proxy_url, ForwardProxyProtocol::Vless)
            .is_empty()
    );
    let stable_trojan_proxy_key =
        normalize_single_proxy_key(explicit_trojan_proxy_url).expect("stable trojan proxy key");
    assert_ne!(explicit_legacy_trojan_proxy_key, stable_trojan_proxy_key);
    assert_ne!(omitted_default_trojan_proxy_key, stable_trojan_proxy_key);

    let mut trojan_manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![explicit_trojan_proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );
    for endpoint in &mut trojan_manager.endpoints {
        endpoint.endpoint_url = Some(
            Url::parse("socks5://127.0.0.1:11081").expect("parse synthesized trojan endpoint url"),
        );
    }
    let trojan_scope = ForwardProxyRouteScope::from_group_binding(
        Some("东京组"),
        vec![omitted_default_trojan_proxy_key.clone()],
    );
    let selected_trojan = trojan_manager
        .select_proxy_for_scope(&trojan_scope)
        .expect("legacy trojan bound key should still select proxy");
    assert_eq!(selected_trojan.key, stable_trojan_proxy_key);
}

#[test]
pub(crate) fn legacy_vless_bound_proxy_keys_still_match_when_query_param_names_change() {
    let legacy_type_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?encryption=none&security=tls&type=ws&host=cdn.example.com&path=%2Fws&sni=edge.example.com&fingerprint=chrome#东京节点";
    let current_net_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?encryption=none&security=tls&net=ws&host=cdn.example.com&path=%2Fws&serverName=edge.example.com&fp=chrome#东京节点";
    let normalized_legacy_type_proxy_url =
        normalize_share_link_scheme(legacy_type_proxy_url, "vless")
            .expect("normalize legacy vless url");
    let normalized_current_net_proxy_url =
        normalize_share_link_scheme(current_net_proxy_url, "vless")
            .expect("normalize current vless url");

    let legacy_bound_proxy_key = {
        let parsed = Url::parse(&normalized_legacy_type_proxy_url)
            .expect("parse legacy normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let stable_proxy_key = normalize_single_proxy_key(current_net_proxy_url)
        .expect("normalize stable vless proxy key");
    assert_ne!(legacy_bound_proxy_key, stable_proxy_key);

    let aliases = legacy_bound_proxy_key_aliases(
        &normalized_current_net_proxy_url,
        ForwardProxyProtocol::Vless,
    );
    assert!(
        aliases.contains(&legacy_bound_proxy_key),
        "legacy alias list should include the historical type=ws key"
    );

    let mut manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![current_net_proxy_url.to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );
    for endpoint in &mut manager.endpoints {
        endpoint.endpoint_url = Some(
            Url::parse("socks5://127.0.0.1:11082")
                .expect("parse synthesized synonym-compatible endpoint url"),
        );
    }

    assert!(
        manager.has_selectable_bound_proxy_keys(std::slice::from_ref(&legacy_bound_proxy_key)),
        "legacy key with synonymous query params should remain selectable"
    );

    let scope =
        ForwardProxyRouteScope::from_group_binding(Some("东京组"), vec![legacy_bound_proxy_key]);
    let selected = manager
        .select_proxy_for_scope(&scope)
        .expect("legacy bound key with synonymous query params should still route");
    assert_eq!(selected.key, stable_proxy_key);
}

#[test]
pub(crate) fn forward_proxy_manager_v2_clamps_persisted_runtime_weight_on_startup() {
    let proxy_url = "http://127.0.0.1:7890".to_string();
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize proxy key");
    let manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![proxy_url.clone()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
        vec![ForwardProxyRuntimeState {
            proxy_key: proxy_key.clone(),
            display_name: proxy_url.clone(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            endpoint_url: Some(proxy_url.clone()),
            weight: 99.0,
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        }],
        ForwardProxyAlgo::V2,
    );

    let manual_runtime = manager
        .runtime
        .get(&proxy_key)
        .expect("manual runtime should exist");
    assert_eq!(manual_runtime.weight, FORWARD_PROXY_V2_WEIGHT_MAX);
}

#[test]
pub(crate) fn forward_proxy_manager_v2_counts_only_selectable_positive_candidates() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![
                "http://127.0.0.1:7890".to_string(),
                "http://127.0.0.1:7891".to_string(),
                "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                    .to_string(),
            ],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    let selectable_keys = manager
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .collect::<Vec<_>>();
    assert_eq!(selectable_keys.len(), 2);
    let non_selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| !endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("non-selectable endpoint should exist");

    manager
        .runtime
        .get_mut(&selectable_keys[0])
        .expect("selectable runtime should exist")
        .weight = 1.0;
    manager
        .runtime
        .get_mut(&selectable_keys[1])
        .expect("selectable runtime should exist")
        .weight = -5.0;
    manager
        .runtime
        .get_mut(&non_selectable_key)
        .expect("non-selectable runtime should exist")
        .weight = 1.0;

    manager.ensure_non_zero_weight();

    let positive_selectable = selectable_keys
        .iter()
        .filter_map(|key| manager.runtime.get(key))
        .filter(|runtime| runtime.weight > 0.0)
        .count();
    assert_eq!(positive_selectable, 2);
}

#[test]
pub(crate) fn forward_proxy_manager_v2_probe_ignores_non_selectable_penalties() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![
                "http://127.0.0.1:7890".to_string(),
                "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                    .to_string(),
            ],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    let selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("selectable endpoint should exist");
    let non_selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| !endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("non-selectable endpoint should exist");

    manager
        .runtime
        .get_mut(&selectable_key)
        .expect("selectable runtime should exist")
        .weight = 1.0;
    manager
        .runtime
        .get_mut(&non_selectable_key)
        .expect("non-selectable runtime should exist")
        .weight = -2.0;

    assert!(!manager.should_probe_penalized_proxy());
    assert!(manager.mark_probe_started().is_none());
}

#[test]
pub(crate) fn forward_proxy_manager_v2_success_with_high_latency_still_gains_weight() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );
    let proxy_key = manager
        .endpoints
        .first()
        .expect("endpoint should exist")
        .key
        .clone();
    let before = manager
        .runtime
        .get(&proxy_key)
        .expect("runtime should exist")
        .weight;

    manager.record_attempt(&proxy_key, true, Some(45_000.0), false);

    let after = manager
        .runtime
        .get(&proxy_key)
        .expect("runtime should exist")
        .weight;
    assert!(after > before, "v2 success should increase weight");
}

use super::*;
