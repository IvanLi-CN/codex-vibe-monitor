use super::*;
use crate::terminal_journal::{TerminalJournal, terminal_journal_directory};
use std::{fs::OpenOptions, io::Write};

#[test]
fn shutdown_recovery_ignores_incomplete_tail_and_accepts_new_records() {
    let root = std::env::temp_dir().join(format!("shutdown-tail-{}", nanoid::nanoid!()));
    fs::create_dir_all(&root).expect("create owned recovery fixture");
    let database_path = root.join("business.sqlite");
    let terminal = recovery_terminal("before-tail");
    TerminalJournal::quarantine_shutdown_batch_at_database_path(
        &database_path,
        &[&terminal],
        &[],
        "test recovery",
    )
    .expect("persist recoverable terminal");
    let path =
        terminal_journal_directory(&database_path).join("shutdown-terminal-quarantine.jsonl");
    let mut file = OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open owned fixture");
    file.write_all(b"{\"invoke_id\":\"incomplete")
        .expect("write interrupted final frame");
    drop(file);
    let mut journal = TerminalJournal::open(&database_path).expect("tail must not prevent startup");
    let writes = journal.take_deferred_writes(32);
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].record.invoke_id, "before-tail");
    assert!(
        !fs::read_to_string(&path)
            .expect("read repaired sink")
            .contains("incomplete")
    );
    let next = recovery_terminal("after-tail");
    journal
        .quarantine_shutdown_batch(&[&next], &[], "new record")
        .expect("append after tail repair");
    drop(journal);
    let mut reopened = TerminalJournal::open(&database_path).expect("restart after appending");
    let mut ids = reopened
        .take_deferred_writes(32)
        .into_iter()
        .map(|write| write.record.invoke_id)
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(ids, ["after-tail", "before-tail"]);
    drop(reopened);
    fs::remove_dir_all(root).expect("remove owned fixture");
}

fn recovery_terminal(id: &str) -> BatchedTerminalInvocationWrite {
    BatchedTerminalInvocationWrite {
        enqueued_at: None,
        diagnostic: None,
        record: crate::tests::test_proxy_capture_record(id, "2026-10-13 12:00:00"),
        attempts: Vec::new(),
        capture_started: None,
        raw_capture: false,
        dashboard_terminal_sequence: None,
        terminal_projection_event_ids: Vec::new(),
        startup_backfill_tasks: Vec::new(),
    }
}

#[test]
fn shutdown_recovery_isolates_middle_damage_and_replays_valid_records() {
    let root = std::env::temp_dir().join(format!("shutdown-middle-{}", nanoid::nanoid!()));
    fs::create_dir_all(&root).expect("create owned fixture");
    let database_path = root.join("business.sqlite");
    let first = recovery_terminal("first");
    let last = recovery_terminal("last");
    TerminalJournal::quarantine_shutdown_batch_at_database_path(
        &database_path,
        &[&first, &last],
        &[],
        "fixture",
    )
    .expect("persist recovery facts");
    let directory = terminal_journal_directory(&database_path);
    let path = directory.join("shutdown-terminal-quarantine.jsonl");
    let valid = fs::read_to_string(&path).expect("read fixture");
    let damaged = valid.replacen('\n', "\n{bad-middle-frame}\n", 1);
    fs::write(&path, &damaged).expect("damage owned fixture");
    let mut journal =
        TerminalJournal::open(&database_path).expect("middle damage must not prevent startup");
    let ids = journal
        .take_deferred_writes(32)
        .into_iter()
        .map(|write| write.record.invoke_id)
        .collect::<Vec<_>>();
    assert_eq!(ids, ["first", "last"]);
    assert!(
        !fs::read_to_string(&path)
            .expect("clean active sink")
            .contains("bad-middle-frame")
    );
    let isolated = fs::read_dir(&directory)
        .expect("read fixture directory")
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("shutdown-terminal-quarantine.jsonl.corrupt-")
        })
        .expect("durable isolated source");
    assert_eq!(
        fs::read_to_string(isolated.path()).expect("read isolated original"),
        damaged
    );
    journal.acknowledge("first", "2026-10-13 12:00:00", false);
    journal.acknowledge("last", "2026-10-13 12:00:00", false);
    drop(journal);
    let reopened = TerminalJournal::open(&database_path).expect("restart after recovery");
    assert_eq!(reopened.stats().pending_materialization_records, 0);
    drop(reopened);
    fs::remove_dir_all(root).expect("remove owned fixture");
}

#[test]
fn shutdown_recovery_cleanup_failure_keeps_source_and_delimits_new_records() {
    let root = std::env::temp_dir().join(format!("shutdown-cleanup-{}", nanoid::nanoid!()));
    fs::create_dir_all(&root).expect("create owned fixture");
    let database_path = root.join("business.sqlite");
    let first = recovery_terminal("first");
    TerminalJournal::quarantine_shutdown_batch_at_database_path(
        &database_path,
        &[&first],
        &[],
        "fixture",
    )
    .expect("persist recovery fact");
    let path =
        terminal_journal_directory(&database_path).join("shutdown-terminal-quarantine.jsonl");
    let mut original = fs::read(&path).expect("read fixture");
    original.extend_from_slice(b"{\"torn");
    fs::write(&path, &original).expect("interrupt owned fixture tail");
    fs::create_dir(path.with_extension("jsonl.tmp")).expect("block only the recovery cleanup sink");
    let mut journal =
        TerminalJournal::open(&database_path).expect("cleanup error must not prevent startup");
    assert_eq!(journal.take_deferred_writes(32).len(), 1);
    assert_eq!(fs::read(&path).expect("retained source"), original);
    journal.acknowledge("first", "2026-10-13 12:00:00", false);
    assert!(
        fs::read(&path)
            .expect("source after ACK")
            .starts_with(&original)
    );
    let next = recovery_terminal("next");
    journal
        .quarantine_shutdown_batch(&[&next], &[], "new fact")
        .expect("append after retained torn frame");
    drop(journal);
    let mut reopened =
        TerminalJournal::open(&database_path).expect("restart with unresolved cleanup error");
    let replay = reopened.take_deferred_writes(32);
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].record.invoke_id, "next");
    drop(reopened);
    fs::remove_dir_all(root).expect("remove owned fixture");
}

#[test]
fn shutdown_recovery_unreadable_source_does_not_disable_main_journal() {
    let root = std::env::temp_dir().join(format!("shutdown-unreadable-{}", nanoid::nanoid!()));
    let database_path = root.join("business.sqlite");
    let path =
        terminal_journal_directory(&database_path).join("shutdown-terminal-quarantine.jsonl");
    fs::create_dir_all(&path).expect("existing non-file recovery source");
    let mut journal = TerminalJournal::open(&database_path)
        .expect("unreadable recovery does not prevent startup");
    let record = test_proxy_capture_record("new-main-event", "2026-10-13 12:00:00");
    assert!(journal.append(&record, false, None).sequence.is_some());
    journal
        .force_sync()
        .expect("new main journal events retain durable confirmation");
    assert!(path.is_dir(), "unreadable original remains untouched");
    drop(journal);
    fs::remove_dir_all(root).expect("remove owned fixture");
}

#[test]
fn shutdown_recovery_preserves_complete_final_record_without_delimiter() {
    let root = std::env::temp_dir().join(format!("shutdown-complete-tail-{}", nanoid::nanoid!()));
    fs::create_dir_all(&root).expect("create owned fixture");
    let database_path = root.join("business.sqlite");
    let terminal = recovery_terminal("complete-tail");
    TerminalJournal::quarantine_shutdown_batch_at_database_path(
        &database_path,
        &[&terminal],
        &[],
        "fixture",
    )
    .expect("persist complete recovery record");
    let path =
        terminal_journal_directory(&database_path).join("shutdown-terminal-quarantine.jsonl");
    let mut content = fs::read(&path).expect("read fixture");
    assert_eq!(content.pop(), Some(b'\n'));
    fs::write(&path, content).expect("remove only the delimiter");
    let mut journal =
        TerminalJournal::open(&database_path).expect("open unterminated complete frame");
    let writes = journal.take_deferred_writes(32);
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].record.invoke_id, "complete-tail");
    drop(journal);
    fs::remove_dir_all(root).expect("remove owned fixture");
}

#[test]
fn shutdown_recovery_ack_failure_keeps_obligation_without_poisoning_main_journal() {
    let root = std::env::temp_dir().join(format!("shutdown-ack-failure-{}", nanoid::nanoid!()));
    fs::create_dir_all(&root).expect("create owned fixture");
    let database_path = root.join("business.sqlite");
    let terminal = recovery_terminal("old-recovery");
    TerminalJournal::quarantine_shutdown_batch_at_database_path(
        &database_path,
        &[&terminal],
        &[],
        "fixture",
    )
    .expect("persist recovery fact");
    let path =
        terminal_journal_directory(&database_path).join("shutdown-terminal-quarantine.jsonl");
    let mut journal = TerminalJournal::open(&database_path).expect("open recovery");
    assert_eq!(journal.take_deferred_writes(32).len(), 1);
    let backup = path.with_extension("jsonl.retained");
    fs::rename(&path, &backup).expect("preserve owned fixture before injecting append failure");
    fs::create_dir(&path).expect("make only the supplemental ACK path unavailable");
    journal.acknowledge("old-recovery", "2026-10-13 12:00:00", false);
    assert!(journal.identity_pending("old-recovery", "2026-10-13 12:00:00", false));
    let record = test_proxy_capture_record("new-main-event", "2026-10-13 12:00:00");
    assert!(journal.append(&record, false, None).sequence.is_some());
    journal
        .force_sync()
        .expect("main journal remains independently durable");
    assert!(backup.is_file());
    drop(journal);
    fs::remove_dir_all(root).expect("remove owned fixture");
}
