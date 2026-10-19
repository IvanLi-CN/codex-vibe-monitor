use super::*;
use crate::sqlite_batch_writer::{
    BatchedTerminalInvocationWrite, SqliteBatchWrite, SqliteBatchWriter,
};

fn summary_terminal(
    invoke_id: &str,
    occurred_at: &str,
    sequence: u64,
) -> BatchedTerminalInvocationWrite {
    let mut record = crate::tests::test_proxy_capture_record(invoke_id, occurred_at);
    record.payload = Some("{}".to_string());
    record.usage.input_tokens = Some(16);
    record.usage.cache_input_tokens = Some(0);
    record.usage.total_tokens = Some(19);
    record.cost = Some(1.5);
    BatchedTerminalInvocationWrite {
        enqueued_at: None,
        attempts: Vec::new(),
        diagnostic: None,
        capture_started: None,
        raw_capture: false,
        dashboard_terminal_sequence: Some(sequence),
        terminal_projection_event_ids: Vec::new(),
        startup_backfill_tasks: Vec::new(),
        record,
    }
}

#[tokio::test]
async fn sqlite_writer_acknowledges_summary_commits_in_sequence_order() {
    let state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let writer = SqliteBatchWriter::spawn_for_test();
    writer.set_auto_flush_terminal_for_test(false);
    let summary_hub = Arc::new(crate::SubscriptionHub::new());
    writer.set_summary_delta_hub(summary_hub.clone());
    let first = summary_terminal("summary-delta-writer-one", "2026-08-09 12:00:00", 1);
    let second = summary_terminal("summary-delta-writer-two", "2026-08-09 12:01:00", 2);

    for terminal in [&first, &second] {
        let record = crate::proxy::api_invocation_from_runtime_record(&terminal.record);
        let mut delta = crate::persisted_dashboard_activity_terminal_delta(&record);
        delta.terminal_sequence = terminal.dashboard_terminal_sequence.expect("sequence");
        summary_hub.register_summary_delta_pending(delta).await;
    }

    assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(second)));
    writer.flush_buffered_for_test(&state.pool).await;
    assert!(
        summary_hub
            .summary_delta_journal_unabsorbed_deltas()
            .await
            .is_empty(),
        "a committed higher sequence remains hidden until its predecessor commits"
    );

    assert!(writer.enqueue(SqliteBatchWrite::TerminalInvocation(first)));
    writer.flush_buffered_for_test(&state.pool).await;
    let deltas = summary_hub.summary_delta_journal_unabsorbed_deltas().await;
    assert_eq!(
        deltas
            .iter()
            .map(|delta| delta.terminal_sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}
