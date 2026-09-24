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
