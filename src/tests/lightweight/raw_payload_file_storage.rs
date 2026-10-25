use super::*;

#[test]
fn archive_batch_file_path_resolves_relative_archive_dir_from_database_parent() {
    let mut config = test_config();
    config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
    config.archive_dir = PathBuf::from("archives");

    let path = archive_batch_file_path(&config, "codex_invocations", "2026-03")
        .expect("resolve archive batch path");

    assert_eq!(
        path,
        PathBuf::from(
            "/tmp/codex-retention/archives/codex_invocations/2026/codex_invocations-2026-03.sqlite.gz",
        )
    );
}

#[test]
fn resolved_proxy_raw_dir_resolves_relative_dir_from_database_parent() {
    let mut config = test_config();
    config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    assert_eq!(
        config.resolved_proxy_raw_dir(),
        PathBuf::from("/tmp/codex-retention/proxy_raw_payloads")
    );
}

#[test]
fn store_raw_payload_file_anchors_relative_dir_to_database_parent() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let temp_dir = make_temp_test_dir("proxy-raw-store-db-parent");
    let cwd = temp_dir.join("cwd");
    let db_root = temp_dir.join("db-root");
    fs::create_dir_all(&cwd).expect("create cwd dir");
    fs::create_dir_all(&db_root).expect("create db root");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd);

    let mut config = test_config();
    config.database_path = db_root.join("codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let meta = runtime.block_on(store_raw_payload_file(
        &config,
        "proxy-test",
        "request",
        Bytes::from_static(b"{\"ok\":true}"),
    ));
    let expected = db_root.join("proxy_raw_payloads/proxy-test-request.bin.zst");

    assert_eq!(
        meta.path.as_deref(),
        Some(expected.to_string_lossy().as_ref())
    );
    assert!(
        expected.exists(),
        "raw payload should be written beside the database"
    );
    assert!(
        !cwd.join("proxy_raw_payloads/proxy-test-request.bin.zst")
            .exists(),
        "raw payload should not follow the current working directory"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn store_raw_payload_file_born_gzips_large_payloads_when_threshold_is_reached() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let temp_dir = make_temp_test_dir("proxy-raw-store-born-gzip");
    let db_root = temp_dir.join("db-root");
    fs::create_dir_all(&db_root).expect("create db root");

    let mut config = test_config();
    config.database_path = db_root.join("codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");
    config.proxy_raw_compression = RawCompressionCodec::Gzip;
    config.proxy_raw_immediate_gzip_bytes = Some(16);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let payload = br#"{"large":"payload-that-should-be-gzipped"}"#;
    let meta = runtime.block_on(store_raw_payload_file(
        &config,
        "proxy-born-gzip",
        "request",
        Bytes::from_static(payload),
    ));

    let path = PathBuf::from(meta.path.expect("born-gzip path"));
    assert!(path.ends_with("proxy-born-gzip-request.bin.gz"));
    assert!(path.exists(), "gzip raw payload should be written");
    let decoded = read_proxy_raw_bytes(path.to_string_lossy().as_ref(), None)
        .expect("read born-gzip payload back");
    assert_eq!(decoded, payload);

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn store_raw_payload_file_born_zstds_identity_payloads() {
    let temp_dir = make_temp_test_dir("proxy-raw-store-born-zstd");
    let mut config = test_config();
    config.database_path = temp_dir.join("codex_vibe_monitor.db");
    config.proxy_raw_compression = RawCompressionCodec::Zstd;
    let payload = br#"{"identity":"stored-with-zstd"}"#;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");
    let meta = runtime.block_on(store_raw_payload_file(
        &config,
        "proxy-born-zstd",
        "response",
        Bytes::from_static(payload),
    ));
    let path = PathBuf::from(meta.path.expect("zstd path"));
    assert!(path.ends_with("proxy-born-zstd-response.bin.zst"));
    assert_eq!(
        read_proxy_raw_bytes(path.to_string_lossy().as_ref(), None).expect("read zstd raw"),
        payload
    );
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn write_streaming_raw_payload_to_file_born_gzips_large_streams() {
    let temp_dir = make_temp_test_dir("proxy-stream-born-gzip");
    let raw_path = temp_dir.join("stream-response.bin");
    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    tx.send(Bytes::from_static(b"hello-"))
        .expect("send first chunk");
    tx.send(Bytes::from_static(b"world-large"))
        .expect("send second chunk");
    drop(tx);

    let meta = write_streaming_raw_payload_to_file(
        raw_path.clone(),
        None,
        Some(8),
        RawCompressionCodec::Gzip,
        &mut rx,
    )
    .await;

    let stored_path = PathBuf::from(meta.path.expect("streaming born-gzip path"));
    assert!(stored_path.ends_with("stream-response.bin.gz"));
    let decoded = read_proxy_raw_bytes(stored_path.to_string_lossy().as_ref(), None)
        .expect("read streaming born-gzip payload");
    assert_eq!(decoded, b"hello-world-large");

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn direct_streaming_raw_payload_writer_respects_gzip_immediate_threshold() {
    let temp_dir = make_temp_test_dir("proxy-direct-stream-gzip-threshold");
    let raw_path = temp_dir.join("stream-response.bin");
    let (tx, rx) = std::sync::mpsc::sync_channel::<Bytes>(2);
    tx.send(Bytes::from_static(b"small"))
        .expect("send small chunk");
    drop(tx);

    let meta = write_direct_streaming_raw_payload_to_file(
        raw_path.clone(),
        None,
        Some(16),
        RawCompressionCodec::Gzip,
        rx,
    );
    let stored_path = PathBuf::from(meta.path.expect("plaintext direct stream path"));
    assert_eq!(stored_path, raw_path);
    assert_eq!(
        read_proxy_raw_bytes(stored_path.to_string_lossy().as_ref(), None)
            .expect("read plaintext direct stream"),
        b"small"
    );

    let compressed_path = temp_dir.join("stream-response-compressed.bin");
    let (tx, rx) = std::sync::mpsc::sync_channel::<Bytes>(2);
    tx.send(Bytes::from_static(b"large"))
        .expect("send large chunk");
    drop(tx);
    let meta = write_direct_streaming_raw_payload_to_file(
        compressed_path.clone(),
        None,
        Some(4),
        RawCompressionCodec::Gzip,
        rx,
    );
    let stored_path = PathBuf::from(meta.path.expect("gzip direct stream path"));
    assert!(stored_path.ends_with("stream-response-compressed.bin.gz"));
    assert_eq!(
        read_proxy_raw_bytes(stored_path.to_string_lossy().as_ref(), None)
            .expect("read gzip direct stream"),
        b"large"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn write_streaming_raw_payload_to_file_removes_plain_path_after_write_failure() {
    use std::os::unix::fs::symlink;

    let temp_dir = make_temp_test_dir("proxy-stream-write-failure-plain");
    let raw_path = temp_dir.join("stream-response.bin");
    symlink("/dev/full", &raw_path).expect("symlink plain raw path to /dev/full");

    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    tx.send(Bytes::from_static(b"hello-world"))
        .expect("send chunk");
    drop(tx);

    let meta = write_streaming_raw_payload_to_file(
        raw_path.clone(),
        None,
        None,
        RawCompressionCodec::None,
        &mut rx,
    )
    .await;

    assert!(meta.path.is_none(), "failed write must not keep raw path");
    assert!(meta.truncated, "failed write should mark payload truncated");
    assert!(
        meta.truncated_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("write_failed")),
        "failed write should surface write_failed reason"
    );
    assert!(
        !raw_path.exists(),
        "failed write should clean up the partially written plain path"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn write_streaming_raw_payload_to_file_removes_gzip_path_after_write_failure() {
    use std::os::unix::fs::symlink;

    let temp_dir = make_temp_test_dir("proxy-stream-write-failure-gzip");
    let raw_path = temp_dir.join("stream-response.bin");
    let gzip_path = temp_dir.join("stream-response.bin.gz");
    symlink("/dev/full", &gzip_path).expect("symlink gzip raw path to /dev/full");

    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    tx.send(Bytes::from_static(b"hello-world"))
        .expect("send chunk");
    drop(tx);

    let meta = write_streaming_raw_payload_to_file(
        raw_path,
        None,
        Some(1),
        RawCompressionCodec::Gzip,
        &mut rx,
    )
    .await;

    assert!(meta.path.is_none(), "failed write must not keep raw path");
    assert!(meta.truncated, "failed write should mark payload truncated");
    assert!(
        meta.truncated_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("write_failed")),
        "failed write should surface write_failed reason"
    );
    assert!(
        !gzip_path.exists(),
        "failed write should clean up the partially written gzip path"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn read_proxy_raw_bytes_keeps_current_dir_compat_for_legacy_relative_paths() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let temp_dir = make_temp_test_dir("proxy-raw-read-legacy-cwd");
    let cwd = temp_dir.join("cwd");
    let fallback_root = temp_dir.join("fallback");
    let relative_path = PathBuf::from("proxy_raw_payloads/legacy-request.bin");
    let cwd_path = cwd.join(&relative_path);
    let fallback_path = fallback_root.join(&relative_path);
    fs::create_dir_all(cwd_path.parent().expect("cwd parent")).expect("create cwd raw dir");
    fs::create_dir_all(fallback_path.parent().expect("fallback parent"))
        .expect("create fallback raw dir");
    fs::write(&cwd_path, b"cwd-copy").expect("write cwd raw file");
    fs::write(&fallback_path, b"fallback-copy").expect("write fallback raw file");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd);

    let raw = read_proxy_raw_bytes(
        relative_path.to_str().expect("utf-8 path"),
        Some(&fallback_root),
    )
    .expect("read legacy cwd-relative raw file");

    assert_eq!(raw, b"cwd-copy");
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn read_proxy_raw_bytes_transparently_decompresses_gzip_files() {
    let temp_dir = make_temp_test_dir("proxy-raw-read-gzip");
    let raw_path = temp_dir.join("request.bin.gz");
    write_gzip_test_file(&raw_path, b"{\"hello\":\"gzip\"}");

    let raw = read_proxy_raw_bytes(raw_path.to_str().expect("utf-8 path"), None)
        .expect("read gzip raw payload");

    assert_eq!(raw, b"{\"hello\":\"gzip\"}");
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn read_proxy_raw_bytes_keeps_plain_bin_payloads_that_start_with_gzip_magic() {
    let temp_dir = make_temp_test_dir("proxy-raw-read-bin-gzip-magic");
    let raw_path = temp_dir.join("request.bin");
    let bytes = vec![0x1f, 0x8b, b'n', b'o', b't', b'-', b'g', b'z'];
    fs::write(&raw_path, &bytes).expect("write plain raw payload");

    let raw = read_proxy_raw_bytes(raw_path.to_str().expect("utf-8 path"), None)
        .expect("read plain raw payload");

    assert_eq!(raw, bytes);
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn search_raw_script_matches_plain_and_gzip_files() {
    let temp_dir = make_temp_test_dir("search-raw-script");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let plain_path = root.join("plain.bin");
    let gzip_path = root.join("cold.bin.gz");
    fs::write(&plain_path, b"line-1\nshared-token\n").expect("write plain raw");
    write_gzip_test_file(&gzip_path, b"line-a\nshared-token\n");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("shared-token")
    .output()
    .expect("run search-raw script");

    assert!(
        output.status.success(),
        "search-raw should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("search-raw stdout");
    assert!(
        stdout.contains(&format!("{}:2:shared-token", plain_path.display())),
        "plain raw file should match, got: {stdout}"
    );
    assert!(
        stdout.contains(&format!("{}:2:shared-token", gzip_path.display())),
        "gzip raw file should match, got: {stdout}"
    );

    let miss_output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("absent-token")
    .output()
    .expect("run search-raw miss case");
    assert_eq!(
        miss_output.status.code(),
        Some(1),
        "search-raw should return 1 when no file matches"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn search_raw_script_resolves_root_from_database_and_proxy_envs() {
    let temp_dir = make_temp_test_dir("search-raw-script-env-root");
    let db_root = temp_dir.join("db");
    let db_path = db_root.join("codex_vibe_monitor.db");
    let raw_root = db_root.join("proxy_raw_payloads");
    fs::create_dir_all(&raw_root).expect("create resolved raw root");
    fs::write(&db_path, "").expect("create db file");
    let plain_path = raw_root.join("resolved.bin");
    fs::write(&plain_path, b"resolved-token\n").expect("write resolved raw");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .env("DATABASE_PATH", &db_path)
    .env("PROXY_RAW_DIR", "proxy_raw_payloads")
    .arg("resolved-token")
    .output()
    .expect("run search-raw with env-derived root");

    assert!(
        output.status.success(),
        "search-raw should resolve root from envs: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("search-raw stdout");
    assert!(
        stdout.contains(&format!("{}:1:resolved-token", plain_path.display())),
        "env-derived root should find the plain file, got: {stdout}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn search_raw_script_reports_missing_root_as_configuration_error() {
    let temp_dir = make_temp_test_dir("search-raw-script-missing-root");
    let db_path = temp_dir.join("missing/codex_vibe_monitor.db");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .env("DATABASE_PATH", &db_path)
    .env("PROXY_RAW_DIR", "proxy_raw_payloads")
    .arg("anything")
    .output()
    .expect("run search-raw with missing root");

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing root should be treated as configuration error"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("root directory not found"),
        "missing root should explain the configuration error, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn spawn_raw_payload_file_write_spools_when_async_writer_pool_is_saturated() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let available_permits = state.proxy_raw_async_semaphore.available_permits();
    let permit = state
        .proxy_raw_async_semaphore
        .clone()
        .acquire_many_owned(available_permits as u32)
        .await
        .expect("saturate async raw writer permits");

    let pending = spawn_raw_payload_file_write(
        state.as_ref(),
        "proxy-test",
        "request",
        Bytes::from_static(br#"{"ok":true}"#),
        true,
    );

    let spool_dir = state.config.resolved_proxy_raw_dir().join(".spool");
    assert!(
        fs::read_dir(&spool_dir)
            .expect("raw spool directory should exist")
            .next()
            .is_some(),
        "saturated capture should be persisted in the overflow spool"
    );

    drop(permit);
    let meta = pending.finish().await;

    assert!(
        meta.path.is_some(),
        "spooled raw payload should be stored after the writer becomes available"
    );
    assert_eq!(meta.size_bytes, br#"{"ok":true}"#.len() as i64);
    assert!(
        !meta.truncated,
        "spooled raw payload should retain its full contents"
    );
    assert!(
        meta.truncated_reason.is_none(),
        "successful spool replay should not report a truncation reason"
    );
}

#[test]
fn search_raw_script_reports_corrupt_gzip_as_error() {
    let temp_dir = make_temp_test_dir("search-raw-script-corrupt-gzip");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let gzip_path = root.join("broken.bin.gz");
    fs::write(&gzip_path, b"not-gzip").expect("write corrupt gzip file");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("needle")
    .output()
    .expect("run search-raw with corrupt gzip");

    assert_eq!(
        output.status.code(),
        Some(2),
        "corrupt gzip should be treated as hard error"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("failed to decompress"),
        "corrupt gzip should explain the decompression failure, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[test]
fn search_raw_script_reports_plain_file_read_errors() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = make_temp_test_dir("search-raw-script-plain-permission-denied");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let plain_path = root.join("plain.bin");
    fs::write(&plain_path, b"permission-token\n").expect("write plain raw");

    let mut permissions = fs::metadata(&plain_path)
        .expect("read plain raw metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&plain_path, permissions).expect("chmod plain raw");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("permission-token")
    .output()
    .expect("run search-raw with unreadable plain file");

    let mut repaired_permissions = fs::metadata(&plain_path)
        .expect("read plain raw metadata after run")
        .permissions();
    repaired_permissions.set_mode(0o644);
    fs::set_permissions(&plain_path, repaired_permissions).expect("restore plain raw permissions");

    assert_eq!(
        output.status.code(),
        Some(2),
        "plain grep errors should be treated as hard errors"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("grep failed"),
        "plain grep failure should be explained, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[test]
fn search_raw_script_reports_find_enumeration_errors() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = make_temp_test_dir("search-raw-script-find-permission-denied");
    let root = temp_dir.join("proxy_raw_payloads");
    let readable_dir = root.join("readable");
    let blocked_dir = root.join("blocked");
    fs::create_dir_all(&readable_dir).expect("create readable raw dir");
    fs::create_dir_all(&blocked_dir).expect("create blocked raw dir");
    fs::write(readable_dir.join("plain.bin"), b"permission-token\n").expect("write readable raw");

    let mut permissions = fs::metadata(&blocked_dir)
        .expect("read blocked dir metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&blocked_dir, permissions).expect("chmod blocked dir");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("permission-token")
    .output()
    .expect("run search-raw with unreadable directory");

    let mut repaired_permissions = fs::metadata(&blocked_dir)
        .expect("read blocked dir metadata after run")
        .permissions();
    repaired_permissions.set_mode(0o755);
    fs::set_permissions(&blocked_dir, repaired_permissions)
        .expect("restore blocked dir permissions");

    assert_eq!(
        output.status.code(),
        Some(2),
        "find enumeration errors should be treated as hard errors"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("failed to enumerate raw files"),
        "find failures should be explained, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}
