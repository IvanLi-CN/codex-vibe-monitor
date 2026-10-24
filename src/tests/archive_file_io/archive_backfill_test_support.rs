use super::*;

pub(super) async fn insert_summary_archive_snapshot_proof(
    pool: &SqlitePool,
    archive_batch_id: i64,
    manifest_sha256: &str,
    coverage_start: &str,
    coverage_end: &str,
    row_count: u32,
) {
    let normalized = serde_json::to_vec(
        &(0..row_count)
            .map(|offset| SummaryArchiveSnapshotV2Record {
                id: i64::from(offset),
                invoke_id: format!("snapshot-proof-{offset}"),
                occurred_at: coverage_start.to_string(),
                source: "proxy".to_string(),
                model: None,
                response_model: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_input_tokens: 0,
                reasoning_tokens: 0,
                reasoning_effort: None,
                total_tokens: 0,
                cost: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                status: "success".to_string(),
                error_message: None,
                failure_kind: None,
                failure_class: None,
                is_actionable: false,
                upstream_account_id: None,
            })
            .collect::<Vec<_>>(),
    )
    .expect("serialize Summary Snapshot proof records");
    let page = SummaryArchiveSnapshotPage {
        archive_batch_id,
        manifest_sha256: manifest_sha256.to_string(),
        page_index: 0,
        coverage_start: coverage_start.to_string(),
        coverage_end: coverage_end.to_string(),
        row_count,
        payload: zstd::stream::encode_all(normalized.as_slice(), 1)
            .expect("compress Summary Snapshot proof records"),
    };
    let mut tx = pool
        .begin()
        .await
        .expect("begin Summary Snapshot proof transaction");
    store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page)
        .await
        .expect("store Summary Snapshot proof");
    tx.commit().await.expect("commit Summary Snapshot proof");
    ensure_summary_archive_snapshot_v2_final_proof(pool, archive_batch_id, manifest_sha256)
        .await
        .expect("commit Summary Snapshot final proof");
}

pub(super) async fn write_valid_invocation_archive(path: &Path, invoke_id: &str) {
    let source_path = PathBuf::from(format!("{}.source.sqlite", path.display()));
    let _ = fs::remove_file(&source_path);
    if let Some(parent) = source_path.parent() {
        fs::create_dir_all(parent).expect("create invocation archive source directory");
    }
    fs::File::create(&source_path).expect("create invocation archive source file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&source_path))
        .await
        .expect("open invocation archive source sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create invocation archive source schema");
    sqlx::query(
        "INSERT INTO codex_invocations (id, invoke_id, occurred_at, raw_response, created_at) \
         VALUES (1, ?1, '2026-01-15 08:15:00', '{}', '2026-01-15 08:15:00')",
    )
    .bind(invoke_id)
    .execute(&archive_pool)
    .await
    .expect("insert invocation archive source row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&source_path, path).expect("compress invocation archive source");
    let _ = fs::remove_file(source_path);
}

pub(super) fn budgeted_archive_io_test_dir(prefix: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-{prefix}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create archive file I/O test directory");
    path
}
