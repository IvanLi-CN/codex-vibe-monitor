use super::*;

pub(crate) fn record(id: i64, occurred_at: &str) -> InvocationAggregateRecord {
    InvocationAggregateRecord {
        id,
        invoke_id: format!("invoke-{id}"),
        occurred_at: occurred_at.to_string(),
        status: Some("success".to_string()),
        total_tokens: Some(3),
        input_tokens: Some(2),
        output_tokens: Some(1),
        cache_input_tokens: Some(1),
        reasoning_tokens: Some(0),
        cost: Some(0.25),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: Some(0),
        live_phase: None,
        t_total_ms: Some(10.0),
        t_req_read_ms: Some(1.0),
        t_req_parse_ms: Some(1.0),
        t_upstream_connect_ms: Some(2.0),
        t_upstream_ttfb_ms: Some(3.0),
        first_token_ms: Some(4.0),
        t_upstream_stream_ms: Some(5.0),
        t_resp_parse_ms: Some(1.0),
        t_persist_ms: Some(1.0),
    }
}
