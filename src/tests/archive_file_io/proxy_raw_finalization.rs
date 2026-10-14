use super::*;

#[tokio::test]
async fn raw_capture_finalization_closes_response_before_waiting_for_queued_request() {
    let fixture = std::env::temp_dir().join(format!("raw-finalization-{}", nanoid::nanoid!()));
    fs::create_dir_all(&fixture).expect("create owned raw finalization fixture");
    let mut config = test_config();
    config.proxy_raw_dir = fixture.join("raw");
    config.proxy_raw_compression = RawCompressionCodec::None;
    let mut state = test_state_from_config(config, true).await;
    Arc::get_mut(&mut state)
        .expect("fixture has one state owner")
        .proxy_raw_async_semaphore = Arc::new(Semaphore::new(1));
    let request_bytes = Bytes::from_static(b"synthetic request");
    let response_bytes = b"synthetic response";
    let mut response = AsyncStreamingRawPayloadWriter::new(
        state.as_ref(),
        "raw-finalization-cycle",
        "response",
        true,
        None,
    );
    response.append(response_bytes);
    assert_eq!(state.proxy_raw_async_semaphore.available_permits(), 0);
    let request = spawn_raw_payload_file_write(
        state.as_ref(),
        "raw-finalization-cycle",
        "request",
        request_bytes.clone(),
        true,
    );
    assert!(matches!(&request, PendingRawPayloadWrite::Task(_)));

    let (request, response, _) = tokio::time::timeout(
        Duration::from_secs(2),
        finish_proxy_raw_captures(request, response),
    )
    .await
    .expect("response EOF must release the slot needed by queued request capture");
    assert!(!request.truncated);
    assert!(!response.truncated);
    let request_path = request.path.expect("complete request capture path");
    let response_path = response.path.expect("complete response capture path");
    assert_ne!(request_path, response_path);
    assert_eq!(
        fs::read(request_path).expect("read request capture"),
        request_bytes
    );
    assert_eq!(
        fs::read(response_path).expect("read response capture"),
        response_bytes
    );
    assert_eq!(state.proxy_raw_async_semaphore.available_permits(), 1);
    state.shutdown.cancel();
    drop(state);
    fs::remove_dir_all(fixture).expect("remove owned raw finalization fixture");
}
