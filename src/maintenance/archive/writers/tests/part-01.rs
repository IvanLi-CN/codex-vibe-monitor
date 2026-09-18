use super::*;

struct TempArchiveTestDirectory(std::path::PathBuf);

impl Drop for TempArchiveTestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn segment_part_key_is_stable_for_one_prepared_identity() {
    let forward = archive_segment_part_key_for_ids(&[7, 3, 5]).expect("part key");
    let reordered = archive_segment_part_key_for_ids(&[5, 7, 3]).expect("part key");
    let distinct = archive_segment_part_key_for_ids(&[3, 5, 8]).expect("part key");

    assert_eq!(forward, reordered);
    assert_ne!(forward, distinct);
}

#[test]
fn prepared_archive_publish_rejects_conflicting_existing_identity() {
    let root = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-archive-publish-{}",
        retention_temp_suffix()
    ));
    fs::create_dir_all(&root).expect("create archive root");
    let temporary = root.join("prepared.sqlite.gz");
    let final_path = root.join("part.sqlite.gz");
    fs::write(&temporary, b"prepared").expect("write prepared archive");
    fs::write(&final_path, b"different").expect("write conflicting archive");

    let error = publish_prepared_archive_file(&temporary, &final_path)
        .expect_err("conflicting archive identity must fail");
    assert!(error.to_string().contains("identity collision"));
    assert!(temporary.exists());
    assert_eq!(
        fs::read(&final_path).expect("read final archive"),
        b"different"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn prepared_archive_publish_reuses_a_matching_identity() {
    let root = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-archive-publish-{}",
        retention_temp_suffix()
    ));
    fs::create_dir_all(&root).expect("create archive root");
    let first_temporary = root.join("first.sqlite.gz");
    let retry_temporary = root.join("retry.sqlite.gz");
    let final_path = root.join("part.sqlite.gz");
    fs::write(&first_temporary, b"prepared").expect("write prepared archive");
    publish_prepared_archive_file(&first_temporary, &final_path)
        .expect("publish first prepared archive");
    fs::write(&retry_temporary, b"prepared").expect("write retry archive");
    publish_prepared_archive_file(&retry_temporary, &final_path)
        .expect("reuse matching prepared archive");

    assert!(!first_temporary.exists());
    assert!(!retry_temporary.exists());
    assert_eq!(
        fs::read(&final_path).expect("read final archive"),
        b"prepared"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn archive_finalization_skips_full_vacuum() {
    let root = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-archive-finalization-{}",
        retention_temp_suffix()
    ));
    let _root_cleanup = TempArchiveTestDirectory(root.clone());
    fs::create_dir_all(&root).expect("create archive root");
    let path = root.join("periodic-maintenance.sqlite");

    let mut connection = open_archive_sqlite_connection(&path)
        .await
        .expect("open archive sqlite file");
    sqlx::query("CREATE TABLE archive_payloads (payload BLOB NOT NULL)")
        .execute(&mut connection)
        .await
        .expect("create archive payload table");
    sqlx::query("INSERT INTO archive_payloads (payload) VALUES (?1)")
        .bind(vec![0_u8; 512 * 1024])
        .execute(&mut connection)
        .await
        .expect("insert archive payload");
    sqlx::query("DELETE FROM archive_payloads")
        .execute(&mut connection)
        .await
        .expect("delete archive payload");
    let free_pages_before: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(&mut connection)
        .await
        .expect("read archive freelist before finalization");
    assert!(
        free_pages_before > 0,
        "fixture must contain reclaimable pages before archive finalization"
    );
    connection.close().await.expect("close archive sqlite file");

    finalize_archive_sqlite_file(&path)
        .await
        .expect("finalize archive sqlite file");

    let mut connection = open_archive_sqlite_connection(&path)
        .await
        .expect("reopen archive sqlite file");
    let free_pages_after: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(&mut connection)
        .await
        .expect("read archive freelist after finalization");
    connection
        .close()
        .await
        .expect("close inspected archive sqlite file");
    let preserved_reclaimable_pages = free_pages_after > 0;

    assert!(
        preserved_reclaimable_pages,
        "periodic archive finalization must not run a full VACUUM"
    );
}

#[tokio::test]
async fn legacy_archive_replacement_keeps_pending_cleanup_when_rename_fails() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let final_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-legacy-archive-replace-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    let missing_temp_path = final_path.with_extension("missing.tmp");
    fs::write(&final_path, b"old archive content").expect("write old archive file");
    let old_sha256 = sha256_hex_file(&final_path).expect("hash old archive file");
    let final_path_string = final_path.to_string_lossy().to_string();
    sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id,
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                cleanup_state,
                cleanup_source_safe_start_date,
                created_at
            )
            VALUES (1, 'codex_quota_snapshots', '2025-01', ?1, ?2, 1, 'completed', 'delete_pending', '2025-01-04', datetime('now'))
            "#,
        )
        .bind(&final_path_string)
        .bind(&old_sha256)
        .execute(&pool)
        .await
        .expect("insert pending archive manifest");

    replace_legacy_archive_file_with_cleanup_serialization(
        &pool,
        "codex_quota_snapshots",
        "2025-01",
        &missing_temp_path,
        &final_path,
    )
    .await
    .expect_err("missing replacement file must roll back pending reactivation");

    assert_eq!(
        fs::read(&final_path).expect("read original archive"),
        b"old archive content"
    );
    let manifest: (String, Option<String>) = sqlx::query_as(
        "SELECT cleanup_state, cleanup_source_safe_start_date FROM archive_batches WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load retained pending manifest");
    assert_eq!(manifest.0, ARCHIVE_CLEANUP_STATE_DELETE_PENDING);
    assert_eq!(manifest.1.as_deref(), Some("2025-01-04"));

    let _ = fs::remove_file(&final_path);
}
