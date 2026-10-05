use super::*;

pub(crate) struct PoolInvocationCleanupGuard {
    state: Arc<AppState>,
    selector: InvocationRecoverySelector,
    recovery_trigger: &'static str,
    prompt_cache_key: Option<String>,
    armed: bool,
    namespace: Option<crate::prompt_cache_conversations::invocation_ranges::lifecycle::PrefixGuard>,
}

impl std::fmt::Debug for PoolInvocationCleanupGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolInvocationCleanupGuard")
            .field("selector", &self.selector)
            .field("recovery_trigger", &self.recovery_trigger)
            .field(
                "prompt_cache_key",
                &self.prompt_cache_key.as_ref().map(|_| "<redacted>"),
            )
            .field("armed", &self.armed)
            .finish()
    }
}

impl PoolInvocationCleanupGuard {
    pub(crate) fn new(
        state: Arc<AppState>,
        selector: InvocationRecoverySelector,
        recovery_trigger: &'static str,
        prompt_cache_key: Option<&str>,
    ) -> Self {
        Self {
            namespace: crate::prompt_cache_conversations::invocation_ranges::lifecycle::PrefixGuard::for_invocation(&selector.invoke_id),
            state,
            selector,
            recovery_trigger,
            prompt_cache_key: prompt_cache_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PoolInvocationCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let state = self.state.clone();
        let selector = self.selector.clone();
        let recovery_trigger = self.recovery_trigger;
        let prompt_cache_key = self.prompt_cache_key.take();
        let namespace = self.namespace.take();
        tokio::spawn(async move {
            if let Err(err) = recover_guard_dropped_pool_invocation_orphan_with_prompt_cache_key(
                state.as_ref(),
                selector,
                recovery_trigger,
                prompt_cache_key,
            )
            .await
            {
                warn!(error = %err, recovery_trigger, "failed to recover dropped pool invocation orphan");
            }
            drop(namespace);
        });
    }
}
