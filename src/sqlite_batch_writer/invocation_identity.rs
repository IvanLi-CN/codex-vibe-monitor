use super::*;

#[derive(Debug)]
pub(crate) struct TrackedTerminalJournal {
    journal: std::sync::Mutex<Option<TerminalJournal>>,
    runtime_attempts: std::sync::Mutex<Option<Arc<RuntimeAttemptStore>>>,
    pub(super) pending:
        crate::prompt_cache_conversations::invocation_ranges::lifecycle::PendingIdentityRegistry,
}

impl TrackedTerminalJournal {
    pub(super) fn new(journal: Option<TerminalJournal>) -> Self {
        let pending = crate::prompt_cache_conversations::invocation_ranges::lifecycle::PendingIdentityRegistry::default();
        if let Some(journal) = &journal {
            for (id, occurred_at, raw) in journal.pending_identity_keys() {
                pending.register(&id, &occurred_at, raw);
            }
        }
        Self {
            journal: std::sync::Mutex::new(journal),
            runtime_attempts: std::sync::Mutex::new(None),
            pending,
        }
    }

    pub(super) fn lock(
        &self,
    ) -> std::sync::LockResult<std::sync::MutexGuard<'_, Option<TerminalJournal>>> {
        self.journal.lock()
    }

    pub(super) fn bind_runtime_attempts(&self, store: Arc<RuntimeAttemptStore>) {
        let mut binding = self
            .runtime_attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if binding.is_none() {
            *binding = Some(store);
        }
    }

    pub(super) fn acknowledge(
        &self,
        id: &str,
        occurred_at: &str,
        raw: bool,
        attempts: &[RuntimeAttemptSnapshot],
    ) {
        // This interface is invoked only after the invocation/attempt transaction
        // commits. Read ownership can be released even if the journal ACK retries.
        if let Some(store) = self
            .runtime_attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            store.acknowledge_materialized(attempts);
        }
        if let Ok(mut guard) = self.lock() {
            if let Some(journal) = guard.as_mut() {
                journal.acknowledge(id, occurred_at, raw);
                if journal.identity_pending(id, occurred_at, raw) {
                    return;
                }
            }
            self.pending.acknowledge(id, occurred_at, raw);
        }
    }
}
