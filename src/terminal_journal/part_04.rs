#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_unacknowledged_record_and_clears_pending_ack() {
        let root = std::env::temp_dir().join(format!("terminal-journal-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record = crate::tests::test_proxy_capture_record("journal-1", "2026-07-29T00:00:00Z");

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        let capture_started = Instant::now()
            .checked_sub(Duration::from_millis(50))
            .expect("capture start should precede now");
        let outcome = journal.append(&record, false, Some(capture_started));
        assert_eq!(
            outcome.durability_mode,
            TerminalJournalDurabilityMode::Journal
        );
        assert_eq!(outcome.pending_records, 1);
        journal.force_sync().expect("sync journal");
        drop(journal);

        let mut replayed = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(replayed.stats().replay_count, 1);
        let replay = replayed.take_replay();
        assert_eq!(replay.len(), 1);
        let replayed_capture_started = replay[0]
            .capture_started
            .expect("replay should preserve raw capture timing");
        assert!(replayed_capture_started.elapsed() >= Duration::from_millis(50));
        replayed.acknowledge("journal-1", "2026-07-29T00:00:00Z", false);
        replayed.force_sync().expect("sync acknowledgement");
        assert_eq!(replayed.stats().pending_records, 0);
        assert!(
            !terminal_journal_directory(&database_path)
                .join("shutdown-terminal-quarantine.jsonl")
                .exists()
        );
        drop(replayed);

        let replayed_after_ack =
            TerminalJournal::open(&database_path).expect("reopen acknowledged journal");
        assert_eq!(replayed_after_ack.stats().replay_count, 0);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn shutdown_recovery_replays_and_compacts_after_acknowledgement() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-shutdown-recovery-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record =
            crate::tests::test_proxy_capture_record("shutdown-recovery-1", "2026-07-29T00:00:00Z");
        let terminal = BatchedTerminalInvocationWrite {
            record: record.clone(),
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
        };

        let journal = TerminalJournal::open(&database_path).expect("open journal");
        TerminalJournal::quarantine_shutdown_batch_at_database_path(
            &database_path,
            &[&terminal],
            &[],
            "test recovery",
        )
        .expect("write shutdown recovery");
        drop(journal);

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen recovery journal");
        assert_eq!(recovered.stats().replay_count, 1);
        assert_eq!(recovered.take_deferred_writes(1).len(), 1);
        recovered.acknowledge(&record.invoke_id, &record.occurred_at, false);
        assert_eq!(recovered.stats().replay_count, 0);
        drop(recovered);

        let recovery_path =
            terminal_journal_directory(&database_path).join("shutdown-terminal-quarantine.jsonl");
        assert!(
            fs::read_to_string(&recovery_path)
                .expect("read compacted recovery")
                .trim()
                .is_empty()
        );
        let reopened = TerminalJournal::open(&database_path).expect("reopen compacted recovery");
        assert_eq!(reopened.stats().replay_count, 0);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn system_task_recovery_replays_across_restart_and_acknowledges() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-system-task-recovery-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let finish = BatchedSystemTaskFinish {
            run_id: 701,
            task_kind: SystemTaskKind::StartupBackfill,
            trigger_kind: "startup".to_string(),
            status: SystemTaskStatus::Skipped,
            summary: Some("cancelled".to_string()),
            detail: Some("shutdown requested".to_string()),
            finished_at: "2026-07-29T00:00:01Z".to_string(),
            duration_ms: 25,
        };

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal
            .quarantine_system_task_finish(&finish, "test recovery")
            .expect("persist system-task recovery");
        drop(journal);

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen journal");
        let replay = recovered.take_system_task_finishes(1);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].run_id, finish.run_id);
        assert_eq!(replay[0].status, finish.status);
        recovered
            .acknowledge_system_task_finishes(&[finish.run_id])
            .expect("acknowledge system-task recovery");
        assert!(recovered.take_system_task_finishes(1).is_empty());
        drop(recovered);

        let mut reopened =
            TerminalJournal::open(&database_path).expect("reopen acknowledged journal");
        assert!(reopened.take_system_task_finishes(1).is_empty());

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn shutdown_system_task_recovery_replays_and_acknowledges_after_restart() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-shutdown-system-task-recovery-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let finish = BatchedSystemTaskFinish {
            run_id: 702,
            task_kind: SystemTaskKind::HourlyRollupBootstrap,
            trigger_kind: "startup".to_string(),
            status: SystemTaskStatus::Skipped,
            summary: Some("cancelled".to_string()),
            detail: None,
            finished_at: "2026-07-29T00:00:02Z".to_string(),
            duration_ms: 40,
        };
        TerminalJournal::quarantine_shutdown_batch_at_database_path(
            &database_path,
            &[],
            &[&finish],
            "shutdown recovery",
        )
        .expect("persist shutdown system-task recovery");

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(recovered.take_system_task_finishes(1).len(), 1);
        recovered
            .acknowledge_system_task_finishes(&[finish.run_id])
            .expect("acknowledge shutdown system-task recovery");
        drop(recovered);

        let mut reopened =
            TerminalJournal::open(&database_path).expect("reopen acknowledged journal");
        assert!(reopened.take_system_task_finishes(1).is_empty());

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn shutdown_system_task_ack_survives_regular_quarantine_compaction() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-shutdown-system-task-compaction-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let finish = BatchedSystemTaskFinish {
            run_id: 702,
            task_kind: SystemTaskKind::HourlyRollupBootstrap,
            trigger_kind: "startup".to_string(),
            status: SystemTaskStatus::Skipped,
            summary: Some("cancelled".to_string()),
            detail: None,
            finished_at: "2026-07-29T00:00:02Z".to_string(),
            duration_ms: 40,
        };
        TerminalJournal::quarantine_shutdown_batch_at_database_path(
            &database_path,
            &[],
            &[&finish],
            "shutdown recovery",
        )
        .expect("persist shutdown system-task recovery");

        let directory = terminal_journal_directory(&database_path);
        let ordinary_path = directory.join("system-task-quarantine.jsonl");
        let mut ordinary = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ordinary_path)
            .expect("open ordinary system-task quarantine");
        for run_id in 10_000_i64..130_000_i64 {
            serde_json::to_writer(
                &mut ordinary,
                &serde_json::json!({ "run_id": run_id, "acknowledged": true }),
            )
            .expect("encode ordinary acknowledgement filler");
            ordinary
                .write_all(b"\n")
                .expect("append ordinary acknowledgement filler");
        }
        ordinary
            .sync_data()
            .expect("sync ordinary acknowledgement filler");
        drop(ordinary);
        assert!(
            fs::metadata(&ordinary_path)
                .expect("stat ordinary system-task quarantine")
                .len()
                >= SYSTEM_TASK_QUARANTINE_COMPACTION_BYTES
        );

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(
            recovered
                .take_system_task_finishes(1)
                .first()
                .map(|finish| finish.run_id),
            Some(finish.run_id)
        );
        recovered
            .acknowledge_system_task_finishes(&[finish.run_id])
            .expect("acknowledge shutdown system-task recovery");
        drop(recovered);

        let shutdown_path = directory.join("shutdown-system-task-quarantine.jsonl");
        assert!(
            fs::read_to_string(&shutdown_path)
                .expect("read compacted shutdown recovery")
                .trim()
                .is_empty()
        );
        let reopened = TerminalJournal::open(&database_path).expect("reopen compacted journal");
        assert!(
            reopened
                .deferred_system_task_finishes
                .iter()
                .all(|entry| entry.run_id != finish.run_id)
        );

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn replay_dispatch_is_bounded_by_the_requested_batch_size() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-replay-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        for index in 0..5 {
            let record = crate::tests::test_proxy_capture_record(
                &format!("journal-replay-{index}"),
                &format!("2026-07-29T00:0{index}:00Z"),
            );
            journal.append(&record, false, None);
        }
        journal.force_sync().expect("sync replay records");
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        reopened.queue_replay_for_dispatch(2);
        assert_eq!(reopened.deferred_write_count(), 2);
        let first_batch = reopened.take_deferred_writes(2);
        assert_eq!(first_batch.len(), 2);
        reopened.queue_replay_for_dispatch(2);
        assert_eq!(reopened.deferred_write_count(), 2);
        let second_batch = reopened.take_deferred_writes(2);
        assert_eq!(second_batch.len(), 2);
        reopened.queue_replay_for_dispatch(2);
        assert_eq!(reopened.deferred_write_count(), 1);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn repairs_corrupt_tail_before_accepting_new_records() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-corrupt-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let first =
            crate::tests::test_proxy_capture_record("journal-corrupt-1", "2026-07-29T00:00:00Z");
        let second =
            crate::tests::test_proxy_capture_record("journal-corrupt-2", "2026-07-29T00:01:00Z");

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&first, false, None);
        journal.force_sync().expect("sync journal");
        let segment_path = journal
            .segments
            .values()
            .find(|segment| segment.current)
            .expect("current segment")
            .path
            .clone();
        drop(journal);
        fs::OpenOptions::new()
            .append(true)
            .open(&segment_path)
            .expect("open journal for corruption")
            .write_all(b"{torn")
            .expect("append corrupt tail");

        let mut repaired = TerminalJournal::open(&database_path).expect("repair journal tail");
        assert_eq!(repaired.stats().replay_count, 1);
        repaired.append(&second, false, None);
        repaired.force_sync().expect("sync repaired journal");
        drop(repaired);

        let reopened = TerminalJournal::open(&database_path).expect("reopen repaired journal");
        assert_eq!(reopened.stats().replay_count, 2);
        let evidence_count = fs::read_dir(terminal_journal_directory(&database_path))
            .expect("read journal directory")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext != "jsonl"))
            .count();
        assert_eq!(evidence_count, 1);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn rotation_open_failure_keeps_existing_segment_current() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-rotation-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        let current_start = journal
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current segment");
        journal
            .segments
            .get_mut(&current_start)
            .expect("current segment")
            .bytes = TERMINAL_JOURNAL_SEGMENT_BYTES;
        journal.directory = root.join("missing-journal-directory");

        assert!(journal.rotate_if_needed(1).is_err());
        assert!(
            journal
                .segments
                .get(&current_start)
                .is_some_and(|segment| segment.current)
        );

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn sync_failure_downgrades_following_appends_to_memory_mode() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-sync-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record =
            crate::tests::test_proxy_capture_record("journal-sync-failed", "2026-07-29T00:00:00Z");
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.sync_failed = true;

        let outcome = journal.append(&record, false, None);
        assert_eq!(
            outcome.durability_mode,
            TerminalJournalDurabilityMode::MemoryOverflow
        );
        assert_eq!(outcome.sequence, None);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn acknowledged_restart_key_does_not_block_a_new_terminal_entry() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-rekey-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record =
            crate::tests::test_proxy_capture_record("journal-rekey", "2026-07-29T00:00:00Z");

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&record, false, None);
        journal.force_sync().expect("sync entry");
        journal.acknowledge(&record.invoke_id, &record.occurred_at, false);
        journal.force_sync().expect("sync acknowledgement");
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(reopened.stats().pending_records, 0);
        let outcome = reopened.append(&record, false, None);
        assert_eq!(
            outcome.durability_mode,
            TerminalJournalDurabilityMode::Journal
        );
        assert!(outcome.sequence.is_some());
        reopened.force_sync().expect("sync replacement entry");
        drop(reopened);

        let reopened = TerminalJournal::open(&database_path).expect("reopen replacement entry");
        assert_eq!(reopened.stats().replay_count, 1);
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn restart_applies_acknowledgement_written_in_a_later_segment() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-cross-ack-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let first =
            crate::tests::test_proxy_capture_record("journal-cross-ack-1", "2026-07-29T00:00:00Z");
        let retained =
            crate::tests::test_proxy_capture_record("journal-cross-ack-2", "2026-07-29T00:01:00Z");
        let current =
            crate::tests::test_proxy_capture_record("journal-cross-ack-3", "2026-07-29T00:02:00Z");

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&first, false, None);
        journal.append(&retained, false, None);
        let first_segment = journal
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("first segment");
        journal
            .segments
            .get_mut(&first_segment)
            .expect("first segment")
            .bytes = TERMINAL_JOURNAL_SEGMENT_BYTES;
        journal.append(&current, false, None);
        let current_segment = journal
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current segment");
        let first_segment_bytes = journal
            .segments
            .get(&first_segment)
            .expect("first segment")
            .bytes;
        let current_segment_bytes = journal
            .segments
            .get(&current_segment)
            .expect("current segment")
            .bytes;

        journal.acknowledge(&first.invoke_id, &first.occurred_at, false);
        assert_eq!(
            journal
                .segments
                .get(&first_segment)
                .expect("first segment")
                .bytes,
            first_segment_bytes
        );
        assert!(
            journal
                .segments
                .get(&current_segment)
                .expect("current segment")
                .bytes
                > current_segment_bytes
        );
        journal.force_sync().expect("sync journal");
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        let replay = reopened.take_replay();
        let replayed_ids = replay
            .iter()
            .map(|write| write.record.invoke_id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(replayed_ids.len(), 2);
        assert!(replayed_ids.contains(retained.invoke_id.as_str()));
        assert!(replayed_ids.contains(current.invoke_id.as_str()));
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn journals_are_isolated_for_databases_in_the_same_directory() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-isolation-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let first_database = root.join("first.db");
        let second_database = root.join("second.db");
        let record =
            crate::tests::test_proxy_capture_record("journal-isolated", "2026-07-29T00:00:00Z");

        let mut first = TerminalJournal::open(&first_database).expect("open first journal");
        first.append(&record, false, None);
        first.force_sync().expect("sync first journal");
        drop(first);

        let second = TerminalJournal::open(&second_database).expect("open second journal");
        assert_eq!(second.stats().replay_count, 0);
        assert_ne!(
            terminal_journal_directory(&first_database),
            terminal_journal_directory(&second_database)
        );
        fs::remove_dir_all(root).expect("remove journal test directory");
    }
}
