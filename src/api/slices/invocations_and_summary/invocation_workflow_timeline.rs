fn build_workflow_timeline_entries(
    record: &ApiInvocation,
    attempts: &[InvocationWorkflowAttempt],
    route_only_attempt: Option<&InvocationWorkflowAttempt>,
    failure_entry: Option<InvocationWorkflowTimelineEntry>,
) -> Vec<InvocationWorkflowTimelineEntry> {
    let mut entries = Vec::new();
    if let Some(route_only_attempt) = route_only_attempt {
        entries.push(build_routing_timeline_entry(
            route_only_attempt
                .attempt_id
                .clone()
                .map(|attempt_id| format!("route-{attempt_id}"))
                .unwrap_or_else(|| "route-terminal".to_string()),
            route_only_attempt,
        ));
    } else if attempts.len() == 1 && attempts[0].synthetic {
        let attempt = attempts[0].clone();
        entries.push(InvocationWorkflowTimelineEntry {
            block_id: "attempt-direct".to_string(),
            kind: "attempt".to_string(),
            occurred_at: Some(attempt.occurred_at.clone()),
            title: "Direct attempt".to_string(),
            subtitle: Some(attempt.endpoint.clone()),
            status: Some(attempt.status.clone()),
            attempt: Some(attempt),
            detail: None,
            response_body: None,
        });
    } else {
        append_workflow_attempt_timeline_entries(&mut entries, attempts);
    }

    if let Some(failure_entry) = failure_entry {
        entries.push(failure_entry);
    }
    if entries.is_empty() && !invocation_status_is_success_like(record) {
        entries.push(InvocationWorkflowTimelineEntry {
            block_id: "failure-only".to_string(),
            kind: "systemFinalFailure".to_string(),
            occurred_at: Some(record.occurred_at.clone()),
            title: "Final downstream response".to_string(),
            subtitle: record.failure_kind.clone(),
            status: record.status.clone(),
            attempt: None,
            detail: Some(json!({
                "downstreamStatusCode": record.downstream_status_code,
                "failureKind": record.failure_kind.clone(),
                "errorMessage": record.error_message.clone(),
                "downstreamErrorMessage": record.downstream_error_message.clone(),
            })),
            response_body: None,
        });
    }
    entries
}

fn append_workflow_attempt_timeline_entries(
    entries: &mut Vec<InvocationWorkflowTimelineEntry>,
    attempts: &[InvocationWorkflowAttempt],
) {
    let mut previous_finished_at: Option<DateTime<Utc>> = None;
    let mut previous_attempt_id: Option<String> = None;
    for attempt in attempts {
        append_workflow_retry_wait_entry(
            entries,
            attempt,
            previous_finished_at,
            previous_attempt_id.as_deref(),
        );
        entries.push(build_routing_timeline_entry(
            format!(
                "route-{}",
                attempt
                    .attempt_id
                    .clone()
                    .unwrap_or_else(|| attempt.attempt_index.to_string())
            ),
            attempt,
        ));
        entries.push(build_workflow_attempt_timeline_entry(attempt.clone()));
        previous_finished_at = attempt
            .finished_at
            .as_deref()
            .and_then(parse_to_utc_datetime);
        previous_attempt_id = attempt.attempt_id.clone();
    }
}

fn append_workflow_retry_wait_entry(
    entries: &mut Vec<InvocationWorkflowTimelineEntry>,
    attempt: &InvocationWorkflowAttempt,
    previous_finished_at: Option<DateTime<Utc>>,
    previous_attempt_id: Option<&str>,
) {
    let Some(started_at) = attempt
        .started_at
        .as_deref()
        .and_then(parse_to_utc_datetime)
    else {
        return;
    };
    let Some(previous_finished) = previous_finished_at else {
        return;
    };
    let gap_ms = (started_at - previous_finished).num_milliseconds();
    if gap_ms <= 0 {
        return;
    }
    entries.push(InvocationWorkflowTimelineEntry {
        block_id: format!(
            "wait-{}",
            attempt
                .attempt_id
                .clone()
                .unwrap_or_else(|| attempt.attempt_index.to_string())
        ),
        kind: "routingWait".to_string(),
        occurred_at: Some(format_utc_iso(started_at)),
        title: "Retry wait".to_string(),
        subtitle: Some(format!("{} ms", gap_ms)),
        status: None,
        attempt: None,
        detail: Some(json!({
            "durationMs": gap_ms,
            "fromAttemptId": previous_attempt_id,
            "toAttemptId": attempt.attempt_id.clone(),
        })),
        response_body: None,
    });
}

fn build_workflow_attempt_timeline_entry(
    attempt: InvocationWorkflowAttempt,
) -> InvocationWorkflowTimelineEntry {
    InvocationWorkflowTimelineEntry {
        block_id: format!(
            "attempt-{}",
            attempt
                .attempt_id
                .clone()
                .unwrap_or_else(|| attempt.attempt_index.to_string())
        ),
        kind: "attempt".to_string(),
        occurred_at: Some(attempt.occurred_at.clone()),
        title: format!("Attempt #{}", attempt.attempt_index),
        subtitle: Some(workflow_attempt_account_label(&attempt)),
        status: Some(attempt.status.clone()),
        attempt: Some(attempt),
        detail: None,
        response_body: None,
    }
}
