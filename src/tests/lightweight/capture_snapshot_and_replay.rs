use super::*;

#[test]
fn should_prebuffer_for_body_sticky_probe_respects_memory_threshold() {
    assert!(should_prebuffer_for_body_sticky_probe(
        false,
        Some("application/json"),
        Some(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES),
    ));
    assert!(!should_prebuffer_for_body_sticky_probe(
        false,
        Some("application/json"),
        Some(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 1),
    ));
    assert!(!should_prebuffer_for_body_sticky_probe(
        true,
        Some("application/json"),
        Some(128),
    ));
}

#[tokio::test]
async fn capture_snapshot_reader_spills_large_body_to_file_and_preserves_bytes() {
    let body_bytes = Bytes::from(vec![
        b'x';
        POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 1024
    ]);

    let snapshot = read_request_body_snapshot_with_partial_limit(
        Body::from(body_bytes.clone()),
        body_bytes.len() + 1024,
        Duration::from_secs(5),
        42,
    )
    .await
    .expect("large body should read into replay snapshot");

    assert_eq!(pool_request_snapshot_kind(&snapshot), "file");
    assert_eq!(
        pool_request_snapshot_body_bytes(&snapshot),
        body_bytes.len()
    );
    assert_eq!(
        snapshot.to_bytes().await.expect("read replay snapshot"),
        body_bytes
    );
}

#[tokio::test]
async fn pool_replay_snapshot_from_bytes_uses_file_for_large_body() {
    let small = Bytes::from(vec![b'a'; POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES]);
    let large = Bytes::from(vec![b'b'; POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 1]);

    let small_snapshot = pool_replay_snapshot_from_bytes(45, small.clone())
        .await
        .expect("build small replay snapshot");
    assert_eq!(pool_request_snapshot_kind(&small_snapshot), "memory");
    assert_eq!(
        small_snapshot
            .to_bytes()
            .await
            .expect("read small snapshot"),
        small
    );

    let large_snapshot = pool_replay_snapshot_from_bytes(46, large.clone())
        .await
        .expect("build large replay snapshot");
    assert_eq!(pool_request_snapshot_kind(&large_snapshot), "file");
    assert_eq!(
        pool_request_snapshot_body_bytes(&large_snapshot),
        large.len()
    );
    assert_eq!(
        large_snapshot
            .to_bytes()
            .await
            .expect("read large snapshot"),
        large
    );
}

#[tokio::test]
async fn pool_replay_snapshot_from_vec_uses_file_for_large_body() {
    let large = vec![b'c'; POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 8];

    let snapshot = pool_replay_snapshot_from_vec(47, large.clone())
        .await
        .expect("build large replay snapshot");

    assert_eq!(pool_request_snapshot_kind(&snapshot), "file");
    assert_eq!(pool_request_snapshot_body_bytes(&snapshot), large.len());
    assert_eq!(
        snapshot.to_bytes().await.expect("read large vec snapshot"),
        Bytes::from(large)
    );
}

#[test]
fn build_pool_replay_temp_path_is_unique_for_same_proxy_request() {
    let first = build_pool_replay_temp_path(48);
    let second = build_pool_replay_temp_path(48);

    assert_ne!(first, second);
}

#[tokio::test]
async fn capture_snapshot_reader_keeps_partial_body_on_limit_error() {
    let err = read_request_body_snapshot_with_partial_limit(
        Body::from(Bytes::from_static(b"abcdef")),
        3,
        Duration::from_secs(5),
        43,
    )
    .await
    .expect_err("body should exceed test limit");

    assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(err.failure_kind, PROXY_FAILURE_BODY_TOO_LARGE);
    assert_eq!(err.partial_body, b"abc");
}

#[tokio::test]
async fn capture_snapshot_reader_bounds_partial_body_on_large_limit_error() {
    let partial_limit = 64 * 1024;
    let body_bytes = Bytes::from(vec![b'x'; partial_limit + 128]);

    let err = read_request_body_snapshot_with_partial_limit(
        Body::from(body_bytes),
        partial_limit + 64,
        Duration::from_secs(5),
        44,
    )
    .await
    .expect_err("body should exceed test limit");

    assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(err.failure_kind, PROXY_FAILURE_BODY_TOO_LARGE);
    assert_eq!(err.partial_body.len(), partial_limit);
}
