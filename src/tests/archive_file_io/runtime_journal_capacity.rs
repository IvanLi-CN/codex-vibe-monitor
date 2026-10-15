use crate::RuntimeAttemptStore;
use crate::terminal_journal::{
    TERMINAL_JOURNAL_MAX_BYTES, TerminalJournal, TerminalJournalDurabilityMode,
    test_attempt_snapshot,
};
use std::fs;

#[test]
fn concurrent_attempts_retain_capacity_for_reverse_order_terminals_and_restart() {
    let root = std::env::temp_dir().join(format!("journal-concurrent-{}", nanoid::nanoid!()));
    fs::create_dir_all(&root).expect("create owned fixture");
    let database_path = root.join("monitor.db");
    let store = RuntimeAttemptStore::default();
    let mut journal = TerminalJournal::open(&database_path).expect("open journal");
    let mut attempts = Vec::new();
    for index in 0..256 {
        let mut attempt = test_attempt_snapshot();
        attempt.attempt_public_id = format!("concurrent-attempt-{index}");
        attempt.invoke_id = format!("concurrent-invocation-{index}");
        assert!(store.upsert(attempt.clone()));
        assert_eq!(
            journal.append_attempt_start(&attempt).durability_mode,
            TerminalJournalDurabilityMode::Journal,
            "begin {index} retains terminal capacity"
        );
        attempts.push(attempt);
    }
    journal.force_sync().expect("confirm accepted begins");
    for mut attempt in attempts.into_iter().rev().take(128) {
        attempt.finished_at = Some("2026-07-29T00:00:01Z".to_string());
        attempt.status = "success".to_string();
        let record =
            crate::tests::test_proxy_capture_record(&attempt.invoke_id, &attempt.occurred_at);
        assert_eq!(
            journal
                .append_with_attempts(&record, &[attempt], false, None)
                .durability_mode,
            TerminalJournalDurabilityMode::Journal
        );
    }
    journal
        .force_sync()
        .expect("confirm reverse-order terminals");
    drop(journal);
    let recovered = TerminalJournal::open(&database_path).expect("recover unfinished begins");
    assert_eq!(recovered.stats().pending_materialization_records, 256);
    assert!(recovered.stats().pending_bytes < TERMINAL_JOURNAL_MAX_BYTES);
    drop(recovered);
    fs::remove_dir_all(&root).expect("remove exact owned fixture");
}
