use super::*;

fn websocket_terminal_write_for_usage(
    invoke_id: &str,
    usage: ParsedUsage,
    cost: f64,
) -> BatchedTerminalInvocationWrite {
    let mut terminal = terminal_write_for_coalescing(invoke_id, None);
    terminal.record.status = "success".to_string();
    terminal.record.usage = usage;
    terminal.record.cost = Some(cost);
    terminal.record.payload = Some(
        mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","streamTerminalEvent":"response.completed"}"#
                .to_string(),
        )
        .expect("mark websocket terminal payload"),
    );
    terminal
}

#[test]
fn websocket_terminal_batch_coalescing_keeps_richer_usage_in_either_order() {
    let poorer = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        total_tokens: Some(1_240),
        ..ParsedUsage::default()
    };
    let richer = ParsedUsage {
        input_tokens: Some(1_200),
        output_tokens: Some(40),
        cache_input_tokens: Some(325),
        reported_cache_write_tokens: Some(50),
        reasoning_tokens: Some(10),
        total_tokens: Some(1_240),
    };

    let mut rich_then_poor = PendingBatch::default();
    rich_then_poor.push(SqliteBatchWrite::TerminalInvocation(
        websocket_terminal_write_for_usage("ws-rich-first", richer.clone(), 0.02),
    ));
    rich_then_poor.push(SqliteBatchWrite::TerminalInvocation(
        websocket_terminal_write_for_usage("ws-rich-first", poorer.clone(), 0.01),
    ));
    let retained = rich_then_poor
        .terminal_invocations
        .values()
        .next()
        .expect("retained websocket terminal");
    assert_eq!(retained.record.usage, richer);
    assert_eq!(retained.record.cost, Some(0.02));

    let mut poor_then_rich = PendingBatch::default();
    poor_then_rich.push(SqliteBatchWrite::TerminalInvocation(
        websocket_terminal_write_for_usage("ws-rich-later", poorer, 0.01),
    ));
    poor_then_rich.push(SqliteBatchWrite::TerminalInvocation(
        websocket_terminal_write_for_usage("ws-rich-later", richer.clone(), 0.02),
    ));
    let retained = poor_then_rich
        .terminal_invocations
        .values()
        .next()
        .expect("upgraded websocket terminal");
    assert_eq!(retained.record.usage, richer);
    assert_eq!(retained.record.cost, Some(0.02));
}

#[test]
fn terminal_batch_coalescing_preserves_the_persistence_ack_sequence() {
    let mut batch = PendingBatch::default();
    let accounting = PendingQueueAccounting::default();
    let mut first = terminal_write_for_coalescing("coalesced-terminal", Some(7));
    first.record.usage.input_tokens = Some(1);
    first.terminal_projection_event_ids.extend(0..64);
    first
        .startup_backfill_tasks
        .push(StartupBackfillTask::ProxyUsage);
    let first = SqliteBatchWrite::TerminalInvocation(first);
    accounting.enqueue(first.estimated_memory_bytes());
    batch.push_accounted(first, &accounting);
    let mut second = terminal_write_for_coalescing("coalesced-terminal", None);
    second.record.usage.input_tokens = Some(2);
    second.terminal_projection_event_ids.extend(64..128);
    second
        .startup_backfill_tasks
        .push(StartupBackfillTask::ReasoningEffort);
    let second = SqliteBatchWrite::TerminalInvocation(second);
    accounting.enqueue(second.estimated_memory_bytes());
    batch.push_accounted(second, &accounting);

    let terminal = batch
        .terminal_invocations
        .values()
        .next()
        .expect("coalesced terminal");
    assert_eq!(terminal.record.usage.input_tokens, Some(2));
    assert_eq!(terminal.dashboard_terminal_sequence, Some(7));
    assert_eq!(
        terminal.terminal_projection_event_ids,
        (0..128).collect::<Vec<_>>()
    );
    assert_eq!(
        terminal.startup_backfill_tasks,
        vec![
            StartupBackfillTask::ReasoningEffort,
            StartupBackfillTask::ProxyUsage,
        ]
    );
    assert_eq!(batch.coalesced_rows, 1);
    assert_eq!(
        batch.estimated_memory_bytes(),
        terminal.estimated_memory_bytes()
    );
    assert_eq!(
        accounting.snapshot().pending_bytes,
        batch.estimated_memory_bytes()
    );
    assert_eq!(accounting.snapshot().pending_depth, batch.logical_rows());
}
