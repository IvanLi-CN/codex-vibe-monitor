use super::*;

pub(super) fn append_terminal_journal(
    terminal_journal: &Arc<TrackedTerminalJournal>,
    terminal: &BatchedTerminalInvocationWrite,
) -> TerminalJournalAppendOutcome {
    let diagnostic_journal = terminal.diagnostic.as_ref().map(|d| {
        d.context
            .waiting(crate::observability::diagnostics::Resource::JournalLock)
    });
    terminal_journal
        .lock()
        .ok()
        .and_then(|mut journal| {
            if let Some(guard) = diagnostic_journal {
                guard.complete();
            }
            journal.as_mut().map(|journal| {
                let append = terminal.diagnostic.as_ref().map(|d| {
                    d.context
                        .phase(crate::observability::diagnostics::Phase::JournalAppend)
                });
                let outcome = journal.append(
                    &terminal.record,
                    terminal.raw_capture,
                    terminal.capture_started,
                );
                if let Some(guard) = append {
                    guard.complete();
                }
                outcome
            })
        })
        .unwrap_or(TerminalJournalAppendOutcome {
            durability_mode: TerminalJournalDurabilityMode::MemoryOverflow,
            sequence: None,
            pending_records: 0,
            pending_bytes: 0,
        })
}

pub(super) fn lock_priority_gate<'a>(
    gate: &'a Arc<std::sync::Mutex<()>>,
    terminal: &BatchedTerminalInvocationWrite,
) -> std::sync::MutexGuard<'a, ()> {
    let diagnostic_priority = terminal.diagnostic.as_ref().map(|d| {
        d.context
            .waiting(crate::observability::diagnostics::Resource::TerminalPriority)
    });
    let guard = gate.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(wait) = diagnostic_priority {
        wait.complete();
    }
    guard
}

pub(super) fn begin_batch(batch: &PendingBatch) -> crate::observability::diagnostics::SharedBatch {
    crate::observability::diagnostics::SharedBatch::begin(
        batch
            .terminal_invocations
            .values()
            .filter_map(|terminal| terminal.diagnostic.as_ref()),
        batch.terminal_invocations.len(),
    )
}

pub(super) fn reference_batch(
    batch: &PendingBatch,
    context: Option<opentelemetry::trace::SpanContext>,
) {
    for terminal in batch.terminal_invocations.values() {
        if let Some(ticket) = &terminal.diagnostic {
            ticket.context.reference_batch(context.clone());
        }
    }
}

pub(super) fn finish_committed_batch(
    batch: &PendingBatch,
    shared: crate::observability::diagnostics::SharedBatch,
) {
    let context = shared.committed();
    for terminal in batch.terminal_invocations.values() {
        if let Some(diagnostic) = &terminal.diagnostic {
            diagnostic.finish_with_batch(
                "committed",
                batch.terminal_invocations.len() > 1,
                context.clone(),
            );
        }
    }
}
