#[cfg(test)]
mod runtime_pressure_health_tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    #[derive(Debug, Default)]
    struct BlockingFilesystemScanTestGate {
        started: AtomicBool,
        released: std::sync::Mutex<bool>,
        wake: std::sync::Condvar,
    }

    impl BlockingFilesystemScanTestGate {
        fn block(&self) {
            self.started.store(true, Ordering::Release);
            let mut released = self
                .released
                .lock()
                .expect("lock filesystem scan test gate");
            while !*released {
                released = self
                    .wake
                    .wait(released)
                    .expect("wait for filesystem scan test gate release");
            }
        }

        fn release(&self) {
            *self
                .released
                .lock()
                .expect("lock filesystem scan test gate") = true;
            self.wake.notify_all();
        }
    }

    async fn wait_for_blocking_filesystem_scan_start(gate: &BlockingFilesystemScanTestGate) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !gate.started.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("filesystem scan should enter the controlled blocking operation");
    }

    async fn wait_for_filesystem_scan_admission_release(in_flight: &AtomicBool) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while in_flight.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("released filesystem scan should free its admission slot");
    }

    async fn assert_deadline_blocked_filesystem_scan_holds_admission(
        state: &Arc<AppState>,
        root: &Path,
        archive_paths: &[String],
        in_flight: &AtomicBool,
    ) {
        let deadline_gate = Arc::new(BlockingFilesystemScanTestGate::default());
        let deadline_gate_for_hook = deadline_gate.clone();
        let deadline_cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_millis(50);
        let deadline_scan =
            SystemStatusFilesystemScan::new(deadline_cancellation.clone(), deadline)
                .with_test_blocking_operation(Arc::new(move || deadline_gate_for_hook.block()));
        let mut deadline_task = {
            let cache = state.system_status_cache.clone();
            let config = state.config.clone();
            let archive_paths = archive_paths.to_vec();
            let root = root.to_path_buf();
            let cancellation = deadline_cancellation.clone();
            tokio::spawn(async move {
                collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
                    SystemStatusFilesystemScanInputs {
                        archive_paths,
                        config,
                        archive_dir: root.clone(),
                        raw_dir: root,
                    },
                    &cache,
                    &cancellation,
                    deadline,
                    deadline_scan,
                )
                .await
            })
        };
        wait_for_blocking_filesystem_scan_start(deadline_gate.as_ref()).await;
        let deadline_result =
            tokio::time::timeout(Duration::from_secs(1), &mut deadline_task).await;
        let held_after_deadline = in_flight.load(Ordering::Acquire);
        let retry_cancellation = CancellationToken::new();
        let retry_deadline = Instant::now() + Duration::from_secs(1);
        let retry_error = collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
            SystemStatusFilesystemScanInputs {
                archive_paths: archive_paths.to_vec(),
                config: state.config.clone(),
                archive_dir: root.to_path_buf(),
                raw_dir: root.to_path_buf(),
            },
            &state.system_status_cache,
            &retry_cancellation,
            retry_deadline,
            SystemStatusFilesystemScan::new(retry_cancellation.clone(), retry_deadline),
        )
        .await
        .expect_err("a detached filesystem scan must retain its only admission slot");
        deadline_gate.release();
        wait_for_filesystem_scan_admission_release(in_flight).await;

        let deadline_error = deadline_result
            .expect("filesystem deadline must return before the blocked syscall is released")
            .expect("filesystem deadline task should join")
            .expect_err("blocked filesystem scan must fail at its deadline");
        assert!(deadline_error.to_string().contains("exceeded its deadline"));
        assert!(
            held_after_deadline,
            "blocked scan must retain the admission slot"
        );
        assert!(retry_error.to_string().contains("already in flight"));
    }

    async fn assert_cancelled_blocked_filesystem_scan_holds_admission(
        state: &Arc<AppState>,
        root: &Path,
        archive_paths: &[String],
        in_flight: &AtomicBool,
    ) {
        let cancellation_gate = Arc::new(BlockingFilesystemScanTestGate::default());
        let cancellation_gate_for_hook = cancellation_gate.clone();
        let cancellation = CancellationToken::new();
        let cancellation_deadline = Instant::now() + Duration::from_secs(1);
        let cancellation_scan =
            SystemStatusFilesystemScan::new(cancellation.clone(), cancellation_deadline)
                .with_test_blocking_operation(Arc::new(move || cancellation_gate_for_hook.block()));
        let mut cancellation_task = {
            let cache = state.system_status_cache.clone();
            let config = state.config.clone();
            let archive_paths = archive_paths.to_vec();
            let root = root.to_path_buf();
            let cancellation = cancellation.clone();
            tokio::spawn(async move {
                collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
                    SystemStatusFilesystemScanInputs {
                        archive_paths,
                        config,
                        archive_dir: root.clone(),
                        raw_dir: root,
                    },
                    &cache,
                    &cancellation,
                    cancellation_deadline,
                    cancellation_scan,
                )
                .await
            })
        };
        wait_for_blocking_filesystem_scan_start(cancellation_gate.as_ref()).await;
        cancellation.cancel();
        let cancellation_result =
            tokio::time::timeout(Duration::from_secs(1), &mut cancellation_task).await;
        let held_after_cancellation = in_flight.load(Ordering::Acquire);
        cancellation_gate.release();
        wait_for_filesystem_scan_admission_release(in_flight).await;

        let cancellation_error = cancellation_result
            .expect("filesystem cancellation must return before the blocked syscall is released")
            .expect("filesystem cancellation task should join")
            .expect_err("blocked filesystem scan must fail when cancelled");
        assert!(cancellation_error.to_string().contains("cancelled"));
        assert!(
            held_after_cancellation,
            "cancelled blocked scan must retain the admission slot"
        );
        assert!(
            state.system_status_cache.lock().await.latest.is_none(),
            "an abandoned filesystem scan must not publish a status snapshot"
        );
    }

    #[test]
    fn active_event_lag_and_writer_pressure_never_report_healthy() {
        assert_eq!(runtime_pressure_state(false, true, false), "degraded");
        assert_eq!(runtime_pressure_state(false, false, true), "deferred");
        assert_eq!(runtime_pressure_state(false, false, false), "healthy");
    }

    #[test]
    fn system_status_refresh_cadence_preserves_lead_before_cache_ceiling() {
        assert_eq!(
            system_status_snapshot_refresh_cadence_period(),
            system_status_snapshot_refresh_delay()
        );
        assert!(
            system_status_snapshot_refresh_cadence_period()
                + SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE
                < Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS)
        );
    }

    #[tokio::test]
    async fn filesystem_scan_deadline_wins_over_ready_result_without_snapshot_write() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let deadline = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("a fresh monotonic instant can move back one millisecond");
        let scan_cancellation = CancellationToken::new();
        let scan_task = tokio::task::spawn_blocking(|| {
            Ok(SystemStatusFilesystemBytes {
                archive_bytes: 1,
                database_bytes: 2,
                other_files_bytes: 3,
            })
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !scan_task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("controlled filesystem scan should complete");

        let error = await_system_status_filesystem_scan_task(
            scan_task,
            &scan_cancellation,
            deadline,
            &Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect_err("a scan completing after its deadline must not escape the select");
        assert!(error.to_string().contains("exceeded its deadline"));

        let publication_cancellation = CancellationToken::new();
        let publication_error = publish_system_status_snapshot(
            state.as_ref(),
            &publication_cancellation,
            deadline,
            SystemStatusResponse {
                live_invocations_count: 0,
                success_count: 0,
                non_success_count: 0,
                completed_archive_batches_count: 0,
                archived_bodies: SystemStatusMetric::default(),
                raw_bodies: SystemStatusMetric::default(),
                request_raw_bodies: SystemStatusMetric::default(),
                response_raw_bodies: SystemStatusMetric::default(),
                database_bytes: 0,
                other_files_bytes: 0,
                projection_health: SystemProjectionHealth::default(),
                raw_metrics_health: SystemRawMetricsHealth::default(),
                runtime_pressure_health: None,
                refreshed_at: String::new(),
            },
            "ready".to_string(),
        )
        .await
        .expect_err("a deadline-expired result must not publish a snapshot");
        assert!(
            publication_error
                .to_string()
                .contains("exceeded its deadline"),
            "the cache lock must recheck the refresh deadline before publication"
        );
        assert!(
            state.system_status_cache.lock().await.latest.is_none(),
            "a rejected filesystem scan result must not write a status snapshot"
        );

        state.pool.close().await;
    }

    #[test]
    fn filesystem_scan_stops_at_deadline_during_directory_traversal() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-system-status-deadline-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("nested")).expect("create deadline scan fixture directory");
        for index in 0..8 {
            fs::write(
                root.join("nested").join(format!("payload-{index}")),
                b"payload",
            )
            .expect("write deadline scan fixture file");
        }

        let checkpoint_count = Arc::new(AtomicUsize::new(0));
        let checkpoint_count_for_hook = checkpoint_count.clone();
        let scan = SystemStatusFilesystemScan::with_test_checkpoint(
            CancellationToken::new(),
            Instant::now() + Duration::from_millis(250),
            Arc::new(move || {
                if checkpoint_count_for_hook.fetch_add(1, Ordering::SeqCst) >= 8 {
                    std::thread::sleep(Duration::from_millis(300));
                }
            }),
        );

        let error = sum_directory_bytes_with_scan(&root, &scan)
            .expect_err("expired filesystem scan must stop the active traversal");
        assert!(error.to_string().contains("exceeded its deadline"));
        assert!(
            checkpoint_count.load(Ordering::SeqCst) > 8,
            "the deadline hook must run after the first file metadata check"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn filesystem_scan_observes_cancellation_during_directory_traversal() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-system-status-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("nested")).expect("create scan fixture directory");
        for index in 0..8 {
            fs::write(
                root.join("nested").join(format!("payload-{index}")),
                b"payload",
            )
            .expect("write scan fixture file");
        }

        let cancellation = CancellationToken::new();
        let checkpoint_count = Arc::new(AtomicUsize::new(0));
        let checkpoint_cancellation = cancellation.clone();
        let checkpoint_count_for_hook = checkpoint_count.clone();
        let scan = SystemStatusFilesystemScan::with_test_checkpoint(
            cancellation,
            Instant::now() + Duration::from_secs(1),
            Arc::new(move || {
                if checkpoint_count_for_hook.fetch_add(1, Ordering::SeqCst) >= 8 {
                    checkpoint_cancellation.cancel();
                }
            }),
        );

        let error = sum_directory_bytes_with_scan(&root, &scan)
            .expect_err("cancellation during traversal must stop the scan");
        assert!(error.to_string().contains("cancelled"));
        assert!(
            checkpoint_count.load(Ordering::SeqCst) > 8,
            "the cancellation hook must run after the first file metadata check"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn filesystem_scan_abandonment_returns_promptly_and_releases_admission() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-system-status-blocking-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create blocking filesystem scan fixture directory");
        let archive_path = root.join("archive.sqlite");
        fs::write(&archive_path, b"archive").expect("write blocking filesystem scan fixture");
        let archive_paths = vec![archive_path.to_string_lossy().to_string()];
        let in_flight = state
            .system_status_cache
            .lock()
            .await
            .filesystem_scan_in_flight
            .clone();

        assert_deadline_blocked_filesystem_scan_holds_admission(
            &state,
            &root,
            &archive_paths,
            in_flight.as_ref(),
        )
        .await;
        assert_cancelled_blocked_filesystem_scan_holds_admission(
            &state,
            &root,
            &archive_paths,
            in_flight.as_ref(),
        )
        .await;

        let _ = fs::remove_dir_all(root);
        state.pool.close().await;
    }

    #[tokio::test]
    async fn system_status_refresh_single_flight_rejects_overlapping_attempts() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let cancellation = CancellationToken::new();
        let flight = SystemStatusSnapshotRefreshFlight::begin(state.as_ref(), &cancellation)
            .await
            .expect("begin first system status refresh");

        let error = SystemStatusSnapshotRefreshFlight::begin(state.as_ref(), &cancellation)
            .await
            .expect_err("overlapping system status refresh must be rejected");
        assert!(error.to_string().contains("already in flight"));

        flight.finish().await;
        state.pool.close().await;
    }

    #[tokio::test]
    async fn stale_status_snapshot_never_falls_back_to_request_sqlite() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        hydrate_system_status_snapshot(state.as_ref())
            .await
            .expect("hydrate status snapshot");
        {
            let mut cache = state.system_status_cache.lock().await;
            cache.latest.as_mut().expect("hydrated entry").cached_at =
                Instant::now() - Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS + 1);
        }
        state.pool.close().await;

        assert!(load_system_status_cached(state.as_ref()).await.is_err());
    }

    #[tokio::test]
    async fn failed_status_refresh_keeps_fresh_last_good_snapshot() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        hydrate_system_status_snapshot(state.as_ref())
            .await
            .expect("hydrate status snapshot");
        let expected = load_system_status_cached(state.as_ref())
            .await
            .expect("load hydrated status snapshot");
        state.pool.close().await;

        assert!(
            refresh_system_status_snapshot(state.as_ref())
                .await
                .is_err()
        );
        assert_eq!(
            load_system_status_cached(state.as_ref())
                .await
                .expect("serve last-good status snapshot"),
            expected
        );
    }
}
