use super::*;

#[tokio::test]
async fn async_streaming_raw_payload_writer_queues_when_global_writer_pool_is_saturated() {
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
async fn raw_overflow_spool_allows_concurrent_captures_without_preallocating_global_capacity() {
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

fn mark_raw_overflow_spool_complete_for_test(spool_dir: &Path) {
    let mut segments = fs::read_dir(spool_dir)
        .expect("read overflow spool directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("frames"))
        .collect::<Vec<_>>();
    segments.sort();
    let segment = segments.last().expect("overflow spool segment");
    let stem = segment
        .file_stem()
        .and_then(|value| value.to_str())
        .expect("overflow spool segment name");
    let (capture_id, segment_index) = stem.rsplit_once('-').expect("capture segment suffix");
    fs::write(spool_dir.join(format!("{capture_id}.done")), segment_index)
        .expect("write completion marker");
}

#[tokio::test]
async fn raw_overflow_spool_recovery_publishes_complete_frames_and_keeps_invalid_files() {
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
    mark_raw_overflow_spool_complete_for_test(&spool_dir);
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
async fn raw_overflow_spool_recovery_retains_unmarked_new_capture() {
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
        "invoke-unmarked-spool",
        "response",
        true,
        None,
    );
    writer.append(b"retain-unmarked");
    drop(writer);

    let spool_dir = state.config.resolved_proxy_raw_dir().join(".spool");
    drop(permits);
    recover_raw_overflow_spools(&state.config).await;

    let recovered = state
        .config
        .resolved_proxy_raw_dir()
        .join("invoke-unmarked-spool-response.bin.zst");
    assert!(
        !recovered.exists(),
        "new-format spool without completion proof must not publish"
    );
    let segments = fs::read_dir(&spool_dir)
        .expect("read overflow spool directory")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("frames"))
        .collect::<Vec<_>>();
    assert!(
        !segments.is_empty(),
        "unmarked spool must remain for a later bounded recovery pass"
    );
    for segment in segments {
        let _ = fs::remove_file(segment.path());
    }
}

#[tokio::test]
async fn raw_overflow_spool_rotates_segments_and_recovers_the_capture_in_order() {
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
    mark_raw_overflow_spool_complete_for_test(&spool_dir);

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
async fn raw_overflow_spool_recovery_retains_a_capture_when_a_later_segment_is_corrupt() {
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
    mark_raw_overflow_spool_complete_for_test(&spool_dir);
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
async fn non_proxy_terminal_transition_marks_timeseries_projection_recovery() {
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
async fn streaming_raw_capture_preserves_precompressed_wire_bytes_without_recompression() {
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
async fn spawn_raw_payload_file_write_skips_new_request_raw_files_when_disabled() {
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
