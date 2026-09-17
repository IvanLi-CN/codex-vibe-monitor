#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite pool");
        sqlx::query(
            "CREATE TABLE summary_source_change_journal (cursor INTEGER PRIMARY KEY AUTOINCREMENT, descriptor_version INTEGER NOT NULL, source_kind TEXT NOT NULL, source_revision INTEGER NOT NULL, first_row_id INTEGER NOT NULL, last_row_id INTEGER NOT NULL, occurred_start TEXT NOT NULL, occurred_end TEXT NOT NULL, descriptor_json TEXT NOT NULL, descriptor_bytes INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("journal table");
        sqlx::query(
            "CREATE TABLE summary_source_change_compaction_proof (id INTEGER PRIMARY KEY AUTOINCREMENT, first_cursor INTEGER NOT NULL, last_cursor INTEGER NOT NULL, proof_kind TEXT NOT NULL, proof_json TEXT NOT NULL, retained_after_cursor INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&pool)
        .await
        .expect("proof table");
        sqlx::query(
            "CREATE TABLE summary_source_change_cursor (scope TEXT PRIMARY KEY, cursor INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("cursor table");
        sqlx::query(
            "CREATE TABLE summary_archive_snapshot (archive_batch_id INTEGER NOT NULL, manifest_sha256 TEXT NOT NULL, page_index INTEGER NOT NULL, coverage_start TEXT NOT NULL, coverage_end TEXT NOT NULL, row_count INTEGER NOT NULL, payload BLOB NOT NULL, payload_bytes INTEGER NOT NULL, snapshot_sha256 TEXT NOT NULL, format_version INTEGER NOT NULL DEFAULT 1, PRIMARY KEY (archive_batch_id, manifest_sha256, page_index))",
        )
        .execute(&pool)
        .await
        .expect("snapshot table");
        sqlx::query(
            "CREATE TABLE archive_batches (id INTEGER PRIMARY KEY, dataset TEXT NOT NULL, sha256 TEXT NOT NULL, row_count INTEGER NOT NULL, status TEXT NOT NULL, coverage_start_at TEXT, coverage_end_at TEXT, file_path TEXT NOT NULL DEFAULT '', summary_source_kind TEXT NOT NULL DEFAULT 'unknown')",
        )
        .execute(&pool)
        .await
        .expect("archive manifest table");
        sqlx::query(
            "CREATE TABLE summary_archive_snapshot_v2_proof (archive_batch_id INTEGER NOT NULL, manifest_sha256 TEXT NOT NULL, page_count INTEGER NOT NULL, row_count INTEGER NOT NULL, coverage_start TEXT, coverage_end TEXT, semantic_sha256 TEXT NOT NULL, verified_at TEXT NOT NULL, PRIMARY KEY (archive_batch_id, manifest_sha256))",
        )
        .execute(&pool)
        .await
        .expect("proof table");
        sqlx::query(
            "CREATE TABLE summary_coverage_obligation (archive_batch_id INTEGER NOT NULL, manifest_sha256 TEXT NOT NULL, coverage_start TEXT, coverage_end TEXT, upstream_account_id INTEGER, current_rank_start INTEGER, current_rank_end INTEGER, state TEXT NOT NULL, terminal_reason TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, resolved_at TEXT, PRIMARY KEY (archive_batch_id, manifest_sha256))",
        )
        .execute(&pool)
        .await
        .expect("obligation table");
        pool
    }

    fn descriptor(row_id: i64) -> SummarySourceChangeDescriptor {
        SummarySourceChangeDescriptor::terminal_batch(
            1,
            vec![SummarySourceChangeEntry {
                row_id,
                invoke_id: format!("invoke-{row_id}"),
                occurred_at: "2026-08-09 12:00:00".to_string(),
                upstream_account_id: Some(7),
                current_rank: Some(1),
            }],
        )
        .expect("descriptor")
    }

    #[tokio::test]
    async fn descriptor_tail_round_trips_and_compacts_with_proof() {
        let pool = pool().await;
        let mut tx = pool.begin().await.expect("transaction");
        append_summary_source_change_descriptor_tx(tx.as_mut(), &descriptor(42))
            .await
            .expect("append descriptor");
        tx.commit().await.expect("commit descriptor");
        let tail = load_summary_source_change_tail(&pool, 0, 10)
            .await
            .expect("load tail");
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].descriptor.entries[0].row_id, 42);
        assert!(
            compact_summary_source_change_journal(&pool, 1, 2)
                .await
                .expect("compact tail")
        );
        let proofs = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM summary_source_change_compaction_proof",
        )
        .fetch_one(&pool)
        .await
        .expect("count proof");
        assert_eq!(proofs, 1);
    }

    #[tokio::test]
    async fn archive_snapshot_cleanup_gate_requires_matching_sha() {
        let pool = pool().await;
        let normalized = serde_json::to_vec(&vec![SummaryArchiveSnapshotV2Record {
            id: 9,
            invoke_id: "invoke-9".to_string(),
            occurred_at: "2026-08-01T00:30:00Z".to_string(),
            source: "proxy".to_string(),
            model: None,
            response_model: None,
            input_tokens: 1,
            output_tokens: 2,
            cache_input_tokens: 0,
            reasoning_tokens: 0,
            reasoning_effort: None,
            total_tokens: 3,
            cost: Some(0.1),
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
        }])
        .expect("serialize V2 snapshot record");
        let page = SummaryArchiveSnapshotPage {
            archive_batch_id: 9,
            manifest_sha256: "manifest-9".to_string(),
            page_index: 0,
            coverage_start: "2026-08-01T00:00:00Z".to_string(),
            coverage_end: "2026-08-01T01:00:00Z".to_string(),
            row_count: 1,
            payload: zstd::stream::encode_all(normalized.as_slice(), 1)
                .expect("compress V2 snapshot record"),
        };
        sqlx::query(
            "INSERT INTO archive_batches (id, dataset, sha256, row_count, status, coverage_start_at, coverage_end_at) VALUES (9, 'codex_invocations', 'manifest-9', 1, 'completed', '2026-08-01T00:00:00Z', '2026-08-01T01:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("store archive manifest");
        let mut tx = pool.begin().await.expect("snapshot transaction");
        store_summary_archive_snapshot_page_tx(tx.as_mut(), &page)
            .await
            .expect("store legacy snapshot");
        tx.commit().await.expect("commit legacy snapshot");
        assert!(
            !summary_archive_snapshot_has_proof(&pool, 9, "manifest-9")
                .await
                .expect("legacy snapshot proof"),
            "V1 pages must never become cleanup authority"
        );
        let mut tx = pool.begin().await.expect("snapshot V2 transaction");
        store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page)
            .await
            .expect("store snapshot");
        tx.commit().await.expect("commit snapshot");
        assert!(
            summary_archive_snapshot_has_proof(&pool, 9, "manifest-9")
                .await
                .expect("snapshot proof")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM summary_archive_snapshot_v2_proof WHERE archive_batch_id = 9",
            )
            .fetch_one(&pool)
            .await
            .expect("count final proof markers"),
            0,
            "a verified page set is not authority until promoted to final proof"
        );
        assert!(
            summary_archive_snapshot_has_final_proof(&pool, 9, "manifest-9")
                .await
                .expect("promote V2 final proof")
        );
        sqlx::query(
            "UPDATE summary_archive_snapshot SET coverage_end = '2026-08-01T02:00:00Z' WHERE archive_batch_id = 9 AND manifest_sha256 = 'manifest-9'",
        )
        .execute(&pool)
        .await
        .expect("mutate V2 page");
        assert!(
            !summary_archive_snapshot_has_final_proof(&pool, 9, "manifest-9")
                .await
                .expect("revoked V2 final proof")
        );
        assert!(
            !summary_archive_snapshot_has_proof(&pool, 9, "other-manifest")
                .await
                .expect("mismatched proof")
        );
    }

    #[tokio::test]
    async fn durable_source_change_checkpoint_is_monotonic() {
        let pool = pool().await;
        store_summary_source_change_checkpoint(&pool, "summary-global", 12)
            .await
            .expect("store first checkpoint");
        store_summary_source_change_checkpoint(&pool, "summary-global", 7)
            .await
            .expect("store stale checkpoint");
        assert_eq!(
            load_summary_source_change_checkpoint(&pool, "summary-global")
                .await
                .expect("load checkpoint"),
            12
        );
    }
}
