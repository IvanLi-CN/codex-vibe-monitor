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

pub(crate) fn websocket_usage_is_strictly_richer(
    previous: &ParsedUsage,
    incoming: &ParsedUsage,
) -> bool {
    let previous_fields = [
        previous.input_tokens,
        previous.output_tokens,
        previous.cache_input_tokens,
        previous.reported_cache_write_tokens,
        previous.reasoning_tokens,
        previous.total_tokens,
    ];
    let incoming_fields = [
        incoming.input_tokens,
        incoming.output_tokens,
        incoming.cache_input_tokens,
        incoming.reported_cache_write_tokens,
        incoming.reasoning_tokens,
        incoming.total_tokens,
    ];

    let preserves_previous_fields = previous_fields
        .iter()
        .zip(incoming_fields)
        .all(|(previous, incoming)| previous.is_none() || *previous == incoming);
    let adds_known_field = previous_fields
        .iter()
        .zip(incoming_fields)
        .any(|(previous, incoming)| previous.is_none() && incoming.is_some());

    preserves_previous_fields && adds_known_field
}

#[derive(Default)]
pub(crate) struct WebSocketUsageAccumulator {
    usage: ParsedUsage,
}

impl WebSocketUsageAccumulator {
    pub(crate) fn update(&mut self, update: ParsedUsage) -> ParsedUsage {
        self.update_with_change(update).0
    }

    pub(crate) fn update_with_change(&mut self, update: ParsedUsage) -> (ParsedUsage, bool) {
        let previous = self.usage.clone();
        self.usage = merge_websocket_usage_update(&self.usage, update);
        (self.usage.clone(), self.usage != previous)
    }

    pub(crate) fn snapshot(&self) -> ParsedUsage {
        self.usage.clone()
    }

    pub(crate) fn reset(&mut self) {
        self.usage = ParsedUsage::default();
    }
}

pub(crate) fn websocket_event_contains_usage(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.created"
            | "response.in_progress"
            | "response.completed"
            | "response.done"
            | "response.failed"
    )
}

pub(crate) fn websocket_event_is_terminal(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.completed" | "response.done" | "response.failed"
    )
}

pub(crate) fn ws_text_event_is_terminal(event_text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(event_text) else {
        return false;
    };
    value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(websocket_event_is_terminal)
}

pub(crate) fn has_ws_usage(usage: &ParsedUsage, failure_without_usage: bool) -> bool {
    failure_without_usage
        || (usage.input_tokens.is_some() && usage.output_tokens.is_some())
        || usage.reported_cache_write_tokens.is_some()
        || usage.cache_input_tokens.is_some()
}

pub(crate) fn has_any_usage_tokens(usage: &ParsedUsage) -> bool {
    usage.total_tokens.is_some()
        || usage.input_tokens.is_some()
        || usage.output_tokens.is_some()
        || usage.cache_input_tokens.is_some()
        || usage.reported_cache_write_tokens.is_some()
        || usage.reasoning_tokens.is_some()
}
