use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::terminal_journal::{TerminalJournal, terminal_journal_directory, test_attempt_snapshot};

// Synthetic 4.5.x frame. The fixed SHA-256 covers the literal [entry,null]
// bytes, independently of the production decoder or a typed re-encoding.
const LEGACY_FLOAT_FRAME: &str = r#"{"entry":{"sequence":1,"raw_capture":false,"capture_elapsed_ms":null,"record":{"invoke_id":"legacy-float","occurred_at":"2026-10-15T00:00:00Z","model":null,"usage":{"input_tokens":null,"output_tokens":null,"cache_input_tokens":null,"reported_cache_write_tokens":null,"reasoning_tokens":null,"total_tokens":null},"cost":0.009548800000000001,"cost_breakdown":{"input":0.0,"cache_write":0.0,"cache_read":0.009548800000000001,"output":0.0,"reasoning":0.0},"cost_estimated":false,"price_version":null,"status":"success","error_message":null,"failure_kind":null,"payload":null,"raw_response":"","response_body_preview_enabled":false,"req_raw":{"path":null,"size_bytes":0,"truncated":false,"truncated_reason":null},"resp_raw":{"path":null,"size_bytes":0,"truncated":false,"truncated_reason":null},"timings":{"t_total_ms":1.0,"t_req_read_ms":0.0,"t_req_parse_ms":0.0,"t_upstream_connect_ms":0.0,"t_upstream_ttfb_ms":0.0,"first_token_ms":null,"t_upstream_stream_ms":0.0,"t_resp_parse_ms":0.0,"t_persist_ms":0.0}}},"acknowledged_sequence":null,"checksum":"7743705be1c75450cfcf0c5a205d6fca06314b6d91c978a7c4279c6836606679"}
"#;

#[test]
fn damaged_middle_frame_is_isolated_without_losing_valid_suffix() {
    let fixture = Fixture::new();
    let first = crate::tests::test_proxy_capture_record("middle-1", "2026-07-29T00:00:00Z");
    let second = crate::tests::test_proxy_capture_record("middle-2", "2026-07-29T00:01:00Z");
    let mut journal = TerminalJournal::open(&fixture.database).expect("open writer");
    journal.append(&first, false, None);
    journal.append(&second, false, None);
    journal.force_sync().expect("sync accepted events");
    drop(journal);
    let original = fs::read(&fixture.segment).expect("read synced events");
    let split = original
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("first frame")
        + 1;
    let mut damaged = original[..split].to_vec();
    damaged.extend_from_slice(b"\xff damaged frame\n");
    damaged.extend_from_slice(&original[split..]);
    fs::write(&fixture.segment, &damaged).expect("inject middle-frame corruption");
    let mut reopened = TerminalJournal::open(&fixture.database).expect("isolate bad middle frame");
    let replay = reopened.take_replay();
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].record.invoke_id, "middle-1");
    assert_eq!(replay[1].record.invoke_id, "middle-2");
    assert_eq!(
        fs::read(&fixture.segment).expect("valid suffix preserved"),
        original
    );
    assert_original_preserved(&fixture.segment, &damaged);
}

#[test]
fn incomplete_tail_is_ignored_before_new_append_and_restart() {
    let fixture = Fixture::new();
    fs::write(&fixture.segment, LEGACY_FLOAT_FRAME).expect("install complete legacy event");
    fs::OpenOptions::new()
        .append(true)
        .open(&fixture.segment)
        .expect("open fixture")
        .write_all(b"{torn")
        .expect("append incomplete tail");
    let original = fs::read(&fixture.segment).expect("read original");
    let mut journal = TerminalJournal::open(&fixture.database).expect("ignore incomplete tail");
    assert_eq!(journal.take_replay().len(), 1);
    assert_original_preserved(&fixture.segment, &original);
    let second = crate::tests::test_proxy_capture_record("after-tail", "2026-07-29T00:01:00Z");
    assert!(journal.append(&second, false, None).sequence.is_some());
    journal
        .force_sync()
        .expect("sync new event after ignored tail");
    drop(journal);
    let mut restarted = TerminalJournal::open(&fixture.database).expect("restart cleaned journal");
    let replay = restarted.take_replay();
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[1].record.invoke_id, "after-tail");
}

#[test]
fn valid_attempt_ack_does_not_create_a_false_interruption_after_isolation() {
    let fixture = Fixture::new();
    let attempt = test_attempt_snapshot();
    let mut record =
        crate::tests::test_proxy_capture_record(&attempt.invoke_id, &attempt.occurred_at);
    record.cost = Some(0.009548800000000001);
    let mut journal = TerminalJournal::open(&fixture.database).expect("open writer");
    journal.append_attempt_start(&attempt);
    journal.append_with_attempts(&record, &[attempt], false, None);
    journal
        .force_sync()
        .expect("sync terminal and its begin ACK");
    journal.acknowledge(&record.invoke_id, &record.occurred_at, false);
    journal.force_sync().expect("sync committed terminal ACK");
    drop(journal);
    let original = fs::read_to_string(&fixture.segment).expect("read synced frames");
    let damaged = original.replace("0.009548800000000001", "0.019548800000000001");
    assert_ne!(original, damaged);
    fs::write(&fixture.segment, &damaged).expect("damage only the committed terminal");
    let mut restarted = TerminalJournal::open(&fixture.database).expect("isolate bad terminal");
    assert!(
        restarted.take_replay().is_empty(),
        "a verified begin ACK must prevent inventing an interrupted terminal over an existing business fact"
    );
    assert_eq!(restarted.stats().pending_records, 0);
    assert_original_preserved(&fixture.segment, damaged.as_bytes());
}

struct Fixture {
    root: PathBuf,
    database: PathBuf,
    segment: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("terminal-recovery-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create owned fixture");
        let database = root.join("monitor.db");
        drop(TerminalJournal::open(&database).expect("create empty journal"));
        let segment =
            terminal_journal_directory(&database).join("terminal-00000000000000000001.jsonl");
        Self {
            root,
            database,
            segment,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove owned fixture");
    }
}

fn assert_original_preserved(path: &Path, original: &[u8]) {
    let prefix = format!(
        "{}.corrupt-",
        path.file_name()
            .expect("segment filename")
            .to_string_lossy()
    );
    let evidence = fs::read_dir(path.parent().expect("segment directory"))
        .expect("read evidence directory")
        .map(|entry| entry.expect("evidence entry").path())
        .find(|candidate| {
            candidate
                .file_name()
                .expect("evidence filename")
                .to_string_lossy()
                .starts_with(&prefix)
        })
        .expect("complete original must be preserved before active replacement");
    assert_eq!(
        fs::read(evidence).expect("read preserved original"),
        original
    );
}

#[test]
fn legacy_float_frame_replays_with_exact_values_and_persistent_ack() {
    let fixture = Fixture::new();
    fs::write(&fixture.segment, LEGACY_FLOAT_FRAME).expect("install synthetic legacy state");
    let mut journal = TerminalJournal::open(&fixture.database).expect("read valid legacy float");
    let replay = journal.take_replay();
    assert_eq!(replay.len(), 1);
    assert_eq!(
        replay[0].record.cost.expect("cost").to_bits(),
        0.009548800000000001_f64.to_bits()
    );
    assert_eq!(
        replay[0]
            .record
            .cost_breakdown
            .expect("breakdown")
            .cache_read
            .to_bits(),
        0.009548800000000001_f64.to_bits()
    );
    assert_eq!(
        fs::read(&fixture.segment).expect("unchanged valid source"),
        LEGACY_FLOAT_FRAME.as_bytes()
    );
    journal.acknowledge("legacy-float", "2026-10-15T00:00:00Z", false);
    journal.force_sync().expect("sync ACK");
    drop(journal);
    let mut restarted = TerminalJournal::open(&fixture.database).expect("re-enter repaired reader");
    assert!(restarted.take_replay().is_empty());
    assert_eq!(restarted.stats().pending_records, 0);
}

#[test]
fn version_one_float_terminal_and_attempt_survive_restart() {
    let fixture = Fixture::new();
    let attempt = test_attempt_snapshot();
    let mut record =
        crate::tests::test_proxy_capture_record(&attempt.invoke_id, &attempt.occurred_at);
    record.cost = Some(0.009548800000000001);
    record.timings.t_total_ms = 42.125;
    let mut journal = TerminalJournal::open(&fixture.database).expect("open writer");
    journal.append_attempt_start(&attempt);
    journal.append_with_attempts(&record, std::slice::from_ref(&attempt), false, None);
    journal.force_sync().expect("sync begin and terminal");
    drop(journal);
    let source = fs::read(&fixture.segment).expect("read version-one frames");
    let envelope: serde_json::Value = serde_json::from_slice(
        source
            .split(|byte| *byte == b'\n')
            .next()
            .expect("first frame"),
    )
    .expect("existing JSON envelope");
    assert_eq!(envelope["event_version"], 1);
    assert!(envelope["attempt_start"].is_object());
    let mut restarted = TerminalJournal::open(&fixture.database).expect("read version-one floats");
    let replay = restarted.take_replay();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].record.cost, record.cost);
    assert_eq!(
        replay[0].record.timings.t_total_ms.to_bits(),
        record.timings.t_total_ms.to_bits()
    );
    assert_eq!(
        replay[0].attempts[0].attempt_public_id,
        attempt.attempt_public_id
    );
    assert_eq!(
        fs::read(&fixture.segment).expect("valid state not rewritten"),
        source
    );
}

#[test]
fn checksum_invalid_complete_frame_is_preserved_and_excluded_from_replay() {
    let fixture = Fixture::new();
    let damaged = LEGACY_FLOAT_FRAME.replace("0.009548800000000001", "0.019548800000000001");
    fs::write(&fixture.segment, &damaged).expect("inject checksum-invalid state");
    let mut journal =
        TerminalJournal::open(&fixture.database).expect("isolate damaged complete frame");
    assert!(journal.take_replay().is_empty());
    assert_original_preserved(&fixture.segment, damaged.as_bytes());
    let record = crate::tests::test_proxy_capture_record("after-isolation", "2026-10-15T00:01:00Z");
    assert!(journal.append(&record, false, None).sequence.is_some());
    journal
        .force_sync()
        .expect("new acceptance remains durable");
    drop(journal);
    let mut restarted = TerminalJournal::open(&fixture.database).expect("read new durable event");
    assert_eq!(
        restarted.take_replay()[0].record.invoke_id,
        "after-isolation"
    );
}

#[test]
fn retained_ack_watermark_cannot_acknowledge_a_reused_sequence() {
    let fixture = Fixture::new();
    let mut source = b"invalid historical frame\n".to_vec();
    source.extend_from_slice(b"{\"entry\":null,\"acknowledged_sequence\":40,\"checksum\":\"80bd9d0a514e625dc55a10f01fb7d0170a864c9a7a6bf1d93dcedd557ef8edd2\"}\n");
    fs::write(&fixture.segment, &source).expect("install damaged segment with valid ACK");
    let mut journal = TerminalJournal::open(&fixture.database).expect("recover valid ACK");
    let record = crate::tests::test_proxy_capture_record("after-ack", "2026-10-15T00:01:00Z");
    assert_eq!(journal.append(&record, false, None).sequence, Some(41));
    journal.force_sync().expect("sync new event");
    drop(journal);
    let mut restarted =
        TerminalJournal::open(&fixture.database).expect("restart above ACK watermark");
    let replay = restarted.take_replay();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].record.invoke_id, "after-ack");
}

#[test]
fn interrupted_cleanup_reenters_from_original_active_source() {
    let fixture = Fixture::new();
    let source = format!("{LEGACY_FLOAT_FRAME}{{torn");
    fs::write(&fixture.segment, &source).expect("retain original before interrupted publication");
    fs::write(
        fixture.segment.with_extension("jsonl.recovery-stopped.tmp"),
        b"incomplete temporary cleanup",
    )
    .expect("simulate interrupted temporary write");
    fs::write(
        fixture.segment.with_extension("jsonl.corrupt-stopped"),
        &source,
    )
    .expect("retain prior complete evidence");
    let mut journal =
        TerminalJournal::open(&fixture.database).expect("re-enter historical cleanup");
    assert_eq!(journal.take_replay().len(), 1);
    assert_eq!(
        fs::read(&fixture.segment).expect("clean source published"),
        LEGACY_FLOAT_FRAME.as_bytes()
    );
    drop(journal);
    let mut restarted =
        TerminalJournal::open(&fixture.database).expect("re-enter published cleanup");
    assert_eq!(restarted.take_replay().len(), 1);
}

#[test]
fn unknown_version_preserves_entire_source_before_any_isolation() {
    let fixture = Fixture::new();
    let mut future: serde_json::Value =
        serde_json::from_str(LEGACY_FLOAT_FRAME).expect("synthetic frame");
    future["event_version"] = serde_json::json!(256);
    let mut source = b"damaged frame before future event\n".to_vec();
    source.extend(serde_json::to_vec(&future).expect("future envelope"));
    source.push(b'\n');
    fs::write(&fixture.segment, &source).expect("install unsupported source");
    let error =
        TerminalJournal::open(&fixture.database).expect_err("future state is not damaged data");
    assert!(
        error
            .to_string()
            .contains("unsupported terminal journal event version")
    );
    assert_eq!(
        fs::read(&fixture.segment).expect("source preserved"),
        source
    );
}
