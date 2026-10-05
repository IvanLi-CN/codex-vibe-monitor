use super::*;

impl WsUsageTracker {
    pub(super) async fn ensure_turn_invoke_id(&mut self, state: &AppState) -> Result<()> {
        if self.active_turn_invoke_id.is_some() {
            return Ok(());
        }
        let turn_prompt_cache_key = self.current_turn_prompt_cache_key().map(ToOwned::to_owned);
        let invoke_id =
            match allocate_proxy_invoke_id(state, turn_prompt_cache_key.as_deref()).await {
                Ok(invoke_id) => invoke_id,
                Err(err) => {
                    warn!(
                        error = %err,
                        trace_invoke_id = %self.trace.invoke_id,
                        prompt_cache_key_fingerprint = self
                            .current_turn_prompt_cache_key()
                            .map(prompt_cache_key_fingerprint),
                        "failed to allocate websocket turn invoke id"
                    );
                    return Err(err);
                }
            };
        self.invocation_leases.insert(
            invoke_id.clone(),
            PromptCacheInvocationLeaseGuard::new(
                state.prompt_cache_conversation_cache.clone(),
                &invoke_id,
            ),
        );
        self.active_turn_invoke_id = Some(invoke_id);
        self.turn_prompt_cache_key = turn_prompt_cache_key.clone();
        if let Some(prompt_cache_key) = turn_prompt_cache_key {
            self.retain_prompt_cache_key(state, &prompt_cache_key).await;
        }
        Ok(())
    }
}
