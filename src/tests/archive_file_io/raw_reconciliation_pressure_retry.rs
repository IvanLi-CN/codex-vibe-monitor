use super::*;

#[tokio::test]
async fn raw_orphan_sweep_retries_pressure_cooldown_inside_task() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-raw-pressure-retry").await;
    let raw_path = config.proxy_raw_dir.join("pressure-retry-orphan.bin");
    fs::write(&raw_path, b"pressure-retry-orphan").expect("write pressure retry orphan");
    let pressure_gate = std::sync::Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        std::time::Duration::from_millis(50),
    ));

    crate::maintenance::RETENTION_TEST_DB_PRESSURE_GATE
        .scope(pressure_gate.clone(), async {
            let first = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
                .await
                .expect("record pressure retry quarantine");
            assert_eq!(first, 0);
            sqlx::query(
                "UPDATE retention_raw_reconciliation SET quarantined_at = ?1 WHERE raw_path = ?2",
            )
            .bind(format_utc_iso(Utc::now() - ChronoDuration::days(2)))
            .bind(raw_path.to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .expect("expire pressure retry quarantine");

            pressure_gate.record_pressure("raw_orphan_sweep_pressure_retry_test", "forced");
            let removed = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
                .await
                .expect("retry raw sweep after pressure cooldown");
            assert_eq!(removed, 1);
            assert!(!raw_path.exists());
        })
        .await;

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
