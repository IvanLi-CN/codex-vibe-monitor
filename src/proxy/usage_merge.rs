use super::ParsedUsage;

pub(crate) fn merge_stream_usage_update(
    previous: &ParsedUsage,
    mut update: ParsedUsage,
) -> ParsedUsage {
    let cache_detail_update_only = update.input_tokens.is_none()
        && update.output_tokens.is_none()
        && update.reasoning_tokens.is_none()
        && (update.cache_input_tokens.is_some() || update.reported_cache_write_tokens.is_some());

    if cache_detail_update_only {
        update.input_tokens = previous.input_tokens;
        update.output_tokens = previous.output_tokens;
        update.cache_input_tokens = update.cache_input_tokens.or(previous.cache_input_tokens);
        update.reasoning_tokens = previous.reasoning_tokens;
        update.total_tokens = update.total_tokens.or(previous.total_tokens);
    }

    update.reported_cache_write_tokens = update
        .reported_cache_write_tokens
        .or(previous.reported_cache_write_tokens);
    update
}

pub(crate) fn merge_websocket_usage_update(
    previous: &ParsedUsage,
    update: ParsedUsage,
) -> ParsedUsage {
    ParsedUsage {
        input_tokens: update.input_tokens.or(previous.input_tokens),
        output_tokens: update.output_tokens.or(previous.output_tokens),
        cache_input_tokens: update.cache_input_tokens.or(previous.cache_input_tokens),
        reported_cache_write_tokens: update
            .reported_cache_write_tokens
            .or(previous.reported_cache_write_tokens),
        reasoning_tokens: update.reasoning_tokens.or(previous.reasoning_tokens),
        total_tokens: update.total_tokens.or(previous.total_tokens),
    }
}

#[derive(Default)]
pub(crate) struct WebSocketUsageAccumulator {
    usage: ParsedUsage,
}

impl WebSocketUsageAccumulator {
    pub(crate) fn update(&mut self, update: ParsedUsage) -> ParsedUsage {
        self.usage = merge_websocket_usage_update(&self.usage, update);
        self.usage.clone()
    }

    pub(crate) fn snapshot(&self) -> ParsedUsage {
        self.usage.clone()
    }

    pub(crate) fn reset(&mut self) {
        self.usage = ParsedUsage::default();
    }
}

pub(crate) fn has_ws_usage(usage: &ParsedUsage, failure_without_usage: bool) -> bool {
    failure_without_usage
        || (usage.input_tokens.is_some() && usage.output_tokens.is_some())
        || usage.reported_cache_write_tokens.is_some()
        || usage.cache_input_tokens.is_some()
}
