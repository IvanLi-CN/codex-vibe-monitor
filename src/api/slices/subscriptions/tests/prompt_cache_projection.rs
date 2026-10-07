use super::*;

#[test]
fn prompt_cache_projection_applies_terminal_delta_without_full_hydration() {
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let mut payload = serde_json::json!({
        "rangeStart": "2026-08-07T00:00:00Z",
        "rangeEnd": "2026-08-08T00:00:00Z",
        "conversations": []
    });
    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.id = 7;
    record.invoke_id = "projection-terminal".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("cache-key".to_string());
    record.total_tokens = Some(42);
    record.cost = Some(0.25);
    record.input_tokens = Some(10);
    record.output_tokens = Some(20);
    record.cache_input_tokens = Some(3);
    record.reported_cache_write_tokens = Some(4);
    record.reasoning_tokens = Some(5);
    record.cost_input = Some(0.01);
    record.cost_cache_write = Some(0.02);
    record.cost_cache_read = Some(0.03);
    record.cost_output = Some(0.04);
    record.cost_reasoning = Some(0.05);
    record.failure_class = Some("none".to_string());

    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build compact delta")
        .expect("prompt cache delta");
    let mut applied_terminal_ids = HashSet::new();
    assert!(
        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            std::slice::from_ref(&delta),
            &mut applied_terminal_ids,
            0,
        )
        .expect("apply terminal delta")
    );
    let conversation = &payload["conversations"][0];
    assert_eq!(conversation["promptCacheKey"], "cache-key");
    assert_eq!(conversation["requestCount"], 1);
    assert_eq!(conversation["totalTokens"], 42);
    assert_eq!(conversation["successCount"], 1);
    assert_eq!(conversation["failureCount"], 0);
    assert_eq!(conversation["inputTokens"], 10);
    assert_eq!(conversation["outputTokens"], 20);
    assert_eq!(conversation["cacheInputTokens"], 3);
    assert_eq!(conversation["reportedCacheWriteTokens"], 4);
    assert_eq!(conversation["reasoningTokens"], 5);
    assert_eq!(conversation["costInput"], 0.01);
    assert_eq!(conversation["costCacheWrite"], 0.02);
    assert_eq!(conversation["costCacheRead"], 0.03);
    assert_eq!(conversation["costOutput"], 0.04);
    assert_eq!(conversation["costReasoning"], 0.05);
    assert_eq!(
        conversation["firstInvocationAt"],
        conversation["lastInvocationAt"]
    );
    assert_eq!(
        conversation["recentInvocations"].as_array().unwrap().len(),
        1
    );
    assert_eq!(conversation["last24hRequests"].as_array().unwrap().len(), 1);

    apply_prompt_cache_records_to_payload(
        &topic,
        &mut payload,
        &[delta],
        &mut applied_terminal_ids,
        0,
    )
    .expect("deduplicate repeated terminal delta");
    assert_eq!(payload["conversations"][0]["requestCount"], 1);
}

#[test]
fn prompt_cache_projection_replay_skips_terminal_preview_already_in_baseline() {
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.id = 0;
    record.invoke_id = "baseline-terminal".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("baseline-key".to_string());
    record.total_tokens = Some(42);
    record.cost = Some(0.25);
    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build baseline replay delta")
        .expect("baseline replay delta");
    let preview = serde_json::to_value(delta.preview.as_ref().expect("baseline preview"))
        .expect("serialize baseline preview");
    let mut payload = serde_json::json!({
        "conversations": [{
            "promptCacheKey": "baseline-key",
            "requestCount": 1,
            "totalTokens": 42,
            "totalCost": 0.25,
            "lastTerminalAt": "2026-08-08T10:00:00Z",
            "recentInvocations": [preview],
            "last24hRequests": []
        }]
    });
    let mut applied_terminal_ids = HashSet::new();

    apply_prompt_cache_records_to_payload(
        &topic,
        &mut payload,
        std::slice::from_ref(&delta),
        &mut applied_terminal_ids,
        0,
    )
    .expect("apply baseline replay delta");

    assert_eq!(payload["conversations"][0]["requestCount"], 1);
    assert!(applied_terminal_ids.contains(&delta.identity));
}

#[test]
fn prompt_cache_projection_preserves_missing_and_sanitizes_invalid_statistics() {
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let occurred_at = "2026-08-08 10:00:00";
    let mut record = dashboard_runtime_topology_live_record(occurred_at);
    record.id = 8;
    record.invoke_id = "projection-invalid-statistics".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("cache-key".to_string());
    record.total_tokens = Some(42);
    record.cost = Some(-0.25);
    record.input_tokens = None;
    record.output_tokens = Some(-20);
    record.cache_input_tokens = Some(3);
    record.reported_cache_write_tokens = None;
    record.reasoning_tokens = Some(5);
    record.cost_input = Some(f64::NAN);
    record.cost_cache_write = Some(-0.02);
    record.cost_cache_read = Some(0.03);
    record.cost_output = Some(f64::INFINITY);
    record.cost_reasoning = Some(0.05);

    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build sanitized delta")
        .expect("prompt cache delta");
    assert_eq!(delta.input_tokens, None);
    assert_eq!(delta.output_tokens, Some(0));
    assert_eq!(delta.cache_input_tokens, Some(3));
    assert_eq!(delta.reported_cache_write_tokens, None);
    assert_eq!(delta.cost, 0.0);
    assert_eq!(delta.cost_input, None);
    assert_eq!(delta.cost_cache_write, Some(0.0));
    assert_eq!(delta.cost_cache_read, Some(0.03));
    assert_eq!(delta.cost_output, None);
    assert_eq!(delta.cost_reasoning, Some(0.05));

    let mut payload = serde_json::json!({
        "conversations": [{
            "promptCacheKey": "cache-key",
            "requestCount": 0,
            "totalTokens": 0,
            "totalCost": 0.0,
            "createdAt": "2026-08-08T09:00:00Z",
            "lastActivityAt": "2026-08-08T09:00:00Z",
            "recentInvocations": [],
            "last24hRequests": []
        }]
    });
    let mut applied_terminal_ids = HashSet::new();
    apply_prompt_cache_records_to_payload(
        &topic,
        &mut payload,
        std::slice::from_ref(&delta),
        &mut applied_terminal_ids,
        0,
    )
    .expect("apply sanitized terminal delta");
    let conversation = &payload["conversations"][0];
    assert!(conversation.get("inputTokens").is_none());
    assert_eq!(conversation["outputTokens"], 0);
    assert_eq!(conversation["cacheInputTokens"], 3);
    assert!(conversation.get("reportedCacheWriteTokens").is_none());
    assert_eq!(conversation["reasoningTokens"], 5);
    assert!(conversation.get("costInput").is_none());
    assert_eq!(conversation["costCacheWrite"], 0.0);
    assert_eq!(conversation["costCacheRead"], 0.03);
    assert!(conversation.get("costOutput").is_none());
    assert_eq!(conversation["costReasoning"], 0.05);
    assert_eq!(conversation["totalCost"], 0.0);

    let mut unknown_payload = serde_json::json!({
        "conversations": [{
            "promptCacheKey": "cache-key",
            "requestCount": 10,
            "totalTokens": 500,
            "totalCost": 1.0,
            "createdAt": "2026-08-01T09:00:00Z",
            "lastActivityAt": "2026-08-08T09:00:00Z",
            "lastTerminalAt": "2026-08-08T09:00:00Z",
            "recentInvocations": [],
            "last24hRequests": []
        }]
    });
    let mut unknown_applied_terminal_ids = HashSet::new();
    apply_prompt_cache_records_to_payload(
        &topic,
        &mut unknown_payload,
        std::slice::from_ref(&delta),
        &mut unknown_applied_terminal_ids,
        0,
    )
    .expect("apply delta to unknown historical statistics");
    let unknown_conversation = &unknown_payload["conversations"][0];
    assert!(unknown_conversation.get("successCount").is_none());
    assert!(unknown_conversation.get("inputTokens").is_none());
    assert!(unknown_conversation.get("firstInvocationAt").is_none());

    let preview = delta.preview.clone().expect("sanitized preview");
    let mut working_state = phase_summary_test_state("cache-key");
    apply_working_conversation_terminal_delta(
        &mut working_state.response.conversations[0],
        &delta,
        &preview,
    );
    let working_conversation = &working_state.response.conversations[0];
    assert_eq!(working_conversation.input_tokens, None);
    assert_eq!(working_conversation.output_tokens, Some(0));
    assert_eq!(working_conversation.cache_input_tokens, Some(3));
    assert_eq!(working_conversation.reported_cache_write_tokens, None);
    assert_eq!(working_conversation.cost_input, None);
    assert_eq!(working_conversation.cost_cache_write, Some(0.0));
    assert_eq!(working_conversation.cost_output, None);

    let mut unknown_working_state = phase_summary_test_state("cache-key");
    unknown_working_state.response.conversations[0].request_count = 10;
    unknown_working_state.response.conversations[0].total_tokens = 500;
    unknown_working_state.response.conversations[0].last_terminal_at =
        Some("2026-08-08T09:00:00Z".to_string());
    apply_working_conversation_terminal_delta(
        &mut unknown_working_state.response.conversations[0],
        &delta,
        &preview,
    );
    let unknown_working_conversation = &unknown_working_state.response.conversations[0];
    assert_eq!(unknown_working_conversation.success_count, None);
    assert_eq!(unknown_working_conversation.input_tokens, None);
    assert_eq!(unknown_working_conversation.first_invocation_at, None);
}

#[test]
fn working_conversations_projection_preserves_stateful_runtime_contract() {
    let now = Utc::now();
    let make_state = |page_size, recent_invocation_limit, blocked_binding_filter| {
        DashboardWorkingConversationsMaterializerState::new(
            PromptCacheConversationsResponse {
                range_start: format_utc_iso(
                    now - ChronoDuration::minutes(
                        SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                    ),
                ),
                range_end: format_utc_iso_precise(now),
                snapshot_at: Some(format_utc_iso_precise(now)),
                selection_mode: PromptCacheConversationSelectionMode::ActivityWindow,
                selected_limit: None,
                selected_activity_hours: None,
                selected_activity_minutes: Some(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
                implicit_filter: PromptCacheConversationImplicitFilter {
                    kind: None,
                    filtered_count: 0,
                },
                total_matched: Some(0),
                has_more: false,
                next_cursor: None,
                conversations: Vec::new(),
            },
            page_size,
            recent_invocation_limit,
            blocked_binding_filter,
        )
    };
    let local_time = |offset_seconds| {
        format_naive(
            (now - ChronoDuration::seconds(offset_seconds))
                .with_timezone(&Shanghai)
                .naive_local(),
        )
    };
    let terminal_delta = |id: i64, invoke_id: &str, prompt_cache_key: &str, offset_seconds| {
        let mut record = dashboard_runtime_topology_live_record(&local_time(offset_seconds));
        record.id = id;
        record.invoke_id = invoke_id.to_string();
        record.prompt_cache_key = Some(prompt_cache_key.to_string());
        record.upstream_account_id = None;
        record.upstream_account_name = None;
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.total_tokens = Some(42);
        record.cost = Some(0.25);
        record.input_tokens = Some(10);
        record.output_tokens = Some(20);
        record.cache_input_tokens = Some(3);
        record.reported_cache_write_tokens = Some(4);
        record.reasoning_tokens = Some(5);
        record.cost_input = Some(0.01);
        record.cost_cache_write = Some(0.02);
        record.cost_cache_read = Some(0.03);
        record.cost_output = Some(0.04);
        record.cost_reasoning = Some(0.05);
        PromptCacheTopicDelta::from_record(&record)
            .expect("build working terminal delta")
            .expect("working terminal delta")
    };
    let seed_hydrated = |state: &mut DashboardWorkingConversationsMaterializerState,
                         prompt_cache_key: &str,
                         occurred_at: &str,
                         total_matched: i64| {
        assert!(state.replace_hydrated_conversation(
            prompt_cache_key,
            Some(PromptCacheConversationResponse {
                prompt_cache_key: prompt_cache_key.to_string(),
                request_count: 0,
                total_tokens: 0,
                total_cost: 0.0,
                created_at: occurred_at.to_string(),
                last_activity_at: occurred_at.to_string(),
                last_terminal_at: None,
                last_in_flight_at: None,
                conversation_id: None,
                success_count: None,
                failure_count: None,
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reported_cache_write_tokens: None,
                reasoning_tokens: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                first_invocation_at: None,
                last_invocation_at: None,
                in_flight_phase_counts: InvocationPhaseCountsResponse::default(),
                cursor: None,
                has_encrypted_session_owner: false,
                encrypted_owner_account_id: None,
                encrypted_owner_account_name: None,
                encrypted_owner_group_name: None,
                manual_binding: None,
                blocked_binding: None,
                upstream_accounts: Vec::new(),
                recent_invocations: Vec::new(),
                last24h_requests: Vec::new(),
            }),
        ));
        assert!(state.set_total_matched(total_matched));
    };

    let mut projection = make_state(1, 16, None);
    let baseline_window = (
        projection.response.range_start.clone(),
        projection.response.range_end.clone(),
        projection.response.snapshot_at.clone(),
    );
    let mut applied_terminal_ids = HashSet::new();
    let mut running = dashboard_runtime_topology_live_record(&local_time(30));
    running.id = 1;
    running.invoke_id = "runtime-to-terminal".to_string();
    running.prompt_cache_key = Some("alpha".to_string());
    running.upstream_account_id = None;
    running.upstream_account_name = None;
    let running = PromptCacheTopicDelta::from_record(&running)
        .expect("build running delta")
        .expect("running delta");
    assert_eq!(
        projection
            .apply_deltas(std::slice::from_ref(&running), &mut applied_terminal_ids, 0)
            .expect("missing key requires bounded hydration"),
        WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(BTreeSet::from([
            "alpha".to_string()
        ]))
    );
    seed_hydrated(&mut projection, "alpha", &running.occurred_at, 1);
    assert_eq!(
        projection
            .apply_deltas(&[running], &mut applied_terminal_ids, 0)
            .expect("apply hydrated running delta"),
        WorkingConversationsProjectionUpdate::Changed
    );
    assert_eq!(
        (
            projection.response.range_start.clone(),
            projection.response.range_end.clone(),
            projection.response.snapshot_at.clone(),
        ),
        baseline_window,
        "live projection updates must retain the cursor-consistent baseline window"
    );

    let mut transient = make_state(20, 16, None);
    let mut transient_ids = HashSet::new();
    let mut transient_record = dashboard_runtime_topology_live_record(&local_time(25));
    transient_record.id = 2;
    transient_record.invoke_id = "runtime-removed".to_string();
    transient_record.prompt_cache_key = Some("transient".to_string());
    let transient_upsert = PromptCacheTopicDelta::from_record(&transient_record)
        .expect("build transient runtime delta")
        .expect("transient runtime delta");
    seed_hydrated(
        &mut transient,
        "transient",
        &transient_upsert.occurred_at,
        1,
    );
    assert_eq!(
        transient
            .apply_deltas(&[transient_upsert], &mut transient_ids, 0)
            .expect("apply transient runtime preview"),
        WorkingConversationsProjectionUpdate::Changed
    );
    let RuntimeMutation::Invocation(transient_removal) =
        RuntimeMutation::invocation(&transient_record, RuntimeMutationKind::RuntimeRemoved)
    else {
        unreachable!("runtime removal must produce an invocation mutation");
    };
    let transient_removal = PromptCacheTopicDelta::from_runtime_mutation(&transient_removal, None)
        .expect("build transient removal delta")
        .expect("transient removal delta");
    assert_eq!(
        transient
            .apply_deltas(&[transient_removal], &mut transient_ids, 0)
            .expect("remove transient runtime preview"),
        WorkingConversationsProjectionUpdate::Changed
    );
    assert!(transient.response.conversations.is_empty());
    assert_eq!(transient.response.total_matched, Some(0));

    let mut old_in_flight = make_state(20, 16, None);
    let mut old_in_flight_ids = HashSet::new();
    let mut old_in_flight_record = dashboard_runtime_topology_live_record(&local_time(16 * 60));
    old_in_flight_record.id = 3;
    old_in_flight_record.invoke_id = "long-running".to_string();
    old_in_flight_record.prompt_cache_key = Some("long-running".to_string());
    let old_in_flight_delta = PromptCacheTopicDelta::from_record(&old_in_flight_record)
        .expect("build old in-flight delta")
        .expect("old in-flight delta");
    seed_hydrated(
        &mut old_in_flight,
        "long-running",
        &old_in_flight_delta.occurred_at,
        1,
    );
    assert_eq!(
        old_in_flight
            .apply_deltas(&[old_in_flight_delta], &mut old_in_flight_ids, 0)
            .expect("apply old in-flight delta"),
        WorkingConversationsProjectionUpdate::Changed
    );
    assert_eq!(old_in_flight.response.conversations.len(), 1);
    assert!(
        !old_in_flight.expire(now),
        "in-flight conversations remain in the working set regardless of age"
    );
    assert_eq!(old_in_flight.response.conversations.len(), 1);

    let terminal = terminal_delta(1, "runtime-to-terminal", "alpha", 30);
    assert_eq!(
        projection
            .apply_deltas(
                std::slice::from_ref(&terminal),
                &mut applied_terminal_ids,
                0
            )
            .expect("replace runtime record with terminal"),
        WorkingConversationsProjectionUpdate::Changed
    );
    let alpha = projection
        .response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "alpha")
        .expect("alpha conversation");
    assert_eq!(alpha.request_count, 1);
    assert_eq!(alpha.total_tokens, 42);
    assert_eq!(alpha.success_count, Some(1));
    assert_eq!(alpha.failure_count, Some(0));
    assert_eq!(alpha.input_tokens, Some(10));
    assert_eq!(alpha.output_tokens, Some(20));
    assert_eq!(alpha.cache_input_tokens, Some(3));
    assert_eq!(alpha.reported_cache_write_tokens, Some(4));
    assert_eq!(alpha.reasoning_tokens, Some(5));
    assert_eq!(alpha.cost_input, Some(0.01));
    assert_eq!(alpha.cost_cache_write, Some(0.02));
    assert_eq!(alpha.cost_cache_read, Some(0.03));
    assert_eq!(alpha.cost_output, Some(0.04));
    assert_eq!(alpha.cost_reasoning, Some(0.05));
    assert!(alpha.last_in_flight_at.is_none());
    assert_eq!(alpha.last24h_requests.len(), 1);
    assert_eq!(alpha.upstream_accounts[0].upstream_account_id, None);
    let same_second_terminal = terminal_delta(4, "same-second-terminal", "alpha", 30);
    assert_eq!(
        projection
            .apply_deltas(&[same_second_terminal], &mut applied_terminal_ids, 0)
            .expect("apply distinct terminal in the same persisted second"),
        WorkingConversationsProjectionUpdate::Changed
    );
    let alpha = projection
        .response
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "alpha")
        .expect("alpha conversation with same-second terminals");
    assert_eq!(alpha.request_count, 2);
    assert_eq!(alpha.total_tokens, 84);
    assert_eq!(alpha.last24h_requests.len(), 2);
    assert_eq!(
        alpha
            .last24h_requests
            .iter()
            .map(|point| point.cumulative_tokens)
            .collect::<Vec<_>>(),
        vec![42, 84],
        "distinct invocations in the same persisted second must retain both chart points",
    );
    assert_eq!(
        projection
            .apply_deltas(&[terminal], &mut applied_terminal_ids, 0)
            .expect("deduplicate terminal replay"),
        WorkingConversationsProjectionUpdate::Unchanged
    );
    assert_eq!(projection.response.conversations[0].request_count, 2);

    let mut alpha_runtime_record = dashboard_runtime_topology_live_record(&local_time(10));
    alpha_runtime_record.id = 3;
    alpha_runtime_record.invoke_id = "alpha-runtime-preview".to_string();
    alpha_runtime_record.prompt_cache_key = Some("alpha".to_string());
    let alpha_runtime_preview = PromptCacheTopicDelta::from_record(&alpha_runtime_record)
        .expect("build alpha runtime preview")
        .expect("alpha runtime preview");
    assert_eq!(
        projection
            .apply_deltas(&[alpha_runtime_preview], &mut applied_terminal_ids, 0)
            .expect("apply alpha runtime preview"),
        WorkingConversationsProjectionUpdate::Changed
    );
    let RuntimeMutation::Invocation(alpha_runtime_removal) =
        RuntimeMutation::invocation(&alpha_runtime_record, RuntimeMutationKind::RuntimeRemoved)
    else {
        unreachable!("runtime removal must produce an invocation mutation");
    };
    let alpha_runtime_removal =
        PromptCacheTopicDelta::from_runtime_mutation(&alpha_runtime_removal, None)
            .expect("build alpha runtime removal")
            .expect("alpha runtime removal");
    let terminal_activity_at = projection.response.conversations[0]
        .last_terminal_at
        .clone()
        .expect("alpha terminal activity");
    assert_eq!(
        projection
            .apply_deltas(&[alpha_runtime_removal], &mut applied_terminal_ids, 0)
            .expect("remove alpha runtime preview"),
        WorkingConversationsProjectionUpdate::Changed
    );
    let alpha = &projection.response.conversations[0];
    assert_eq!(alpha.last_in_flight_at, None);
    assert_eq!(alpha.last_activity_at, terminal_activity_at);

    let mut account_history = make_state(20, 16, None);
    let mut account_history_ids = HashSet::new();
    let account_history_delta = terminal_delta(200, "account-history-base", "account-history", 3);
    seed_hydrated(
        &mut account_history,
        "account-history",
        &account_history_delta.occurred_at,
        1,
    );
    account_history.response.conversations[0].upstream_accounts = (1..=3)
        .map(
            |upstream_account_id| PromptCacheConversationUpstreamAccountResponse {
                upstream_account_id: Some(upstream_account_id),
                upstream_account_name: Some(format!("Historical {upstream_account_id}")),
                request_count: 10,
                total_tokens: 100,
                total_cost: 1.0,
                last_activity_at: account_history_delta.occurred_at.clone(),
            },
        )
        .collect();
    let mut omitted_account_delta =
        terminal_delta(201, "omitted-account-live", "account-history", 2);
    omitted_account_delta.upstream_account_id = Some(4);
    omitted_account_delta.upstream_account_name = Some("Historical 4".to_string());
    assert_eq!(
        account_history
            .apply_deltas(&[omitted_account_delta], &mut account_history_ids, 0,)
            .expect("omitted historical account requires bounded hydration"),
        WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(BTreeSet::from([
            "account-history".to_string()
        ]))
    );
    assert_eq!(
        account_history.response.conversations[0]
            .upstream_accounts
            .len(),
        PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT,
        "a partial live delta must not replace a capped historical account summary",
    );

    let beta = terminal_delta(2, "newer-terminal", "beta", 5);
    seed_hydrated(&mut projection, "beta", &beta.occurred_at, 2);
    assert_eq!(
        projection
            .apply_deltas(&[beta], &mut applied_terminal_ids, 0)
            .expect("apply newer page candidate"),
        WorkingConversationsProjectionUpdate::Changed
    );
    assert_eq!(projection.response.total_matched, Some(2));
    assert!(projection.response.has_more);
    assert_eq!(projection.response.conversations.len(), 1);
    assert_eq!(
        projection.response.conversations[0].prompt_cache_key,
        "beta"
    );
    assert!(
        projection.response.next_cursor.is_none(),
        "a live page replacement must not manufacture a cursor for the cold baseline"
    );
    assert!(
        projection.response.conversations[0].cursor.is_none(),
        "a live-only page member has no valid cursor in the cold baseline"
    );

    let binding = PromptCacheConversationBindingResponse {
        prompt_cache_key: "beta".to_string(),
        binding_kind: "upstream_account".to_string(),
        group_name: None,
        upstream_account_id: Some(9),
        upstream_account_name: Some("Pinned account".to_string()),
        has_encrypted_session_owner: true,
        encrypted_owner_account_id: Some(11),
        encrypted_owner_account_name: Some("Owner account".to_string()),
        encrypted_owner_group_name: Some("Owner group".to_string()),
        sticky_routes: Vec::new(),
        timeouts: RoutingTimeoutSettings::default(),
        timeout_field_sources: RoutingTimeoutFieldSources {
            responses_first_byte_timeout_secs: "root".to_string(),
            compact_first_byte_timeout_secs: "root".to_string(),
            image_first_byte_timeout_secs: "root".to_string(),
            responses_stream_timeout_secs: "root".to_string(),
            compact_stream_timeout_secs: "root".to_string(),
        },
        allow_switch_upstream: None,
        fast_mode_rewrite_mode: None,
        image_tool_rewrite_mode: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        forward_proxy_key: None,
        forward_proxy_keys: Vec::new(),
        policy_field_sources: PromptCacheConversationPolicyFieldSources {
            allow_switch_upstream: "root".to_string(),
            fast_mode_rewrite_mode: "root".to_string(),
            image_tool_rewrite_mode: "root".to_string(),
            codex_imagegen_rewrite_mode: "root".to_string(),
            available_models: "root".to_string(),
            available_models_mode: "root".to_string(),
            forward_proxy_key: "root".to_string(),
        },
        updated_at: None,
    };
    assert_eq!(projection.apply_binding("beta", &binding), Some(true));
    let beta = &projection.response.conversations[0];
    assert_eq!(beta.encrypted_owner_account_id, Some(11));
    assert_eq!(
        beta.manual_binding
            .as_ref()
            .map(|value| value.upstream_account_id),
        Some(Some(9))
    );

    let blocked_filter = PromptCacheConversationBlockedBindingFilter {
        upstream_account_id: Some(7),
        constraint_source: Some(BlockedBindingConstraintSource::UpstreamAccountBinding),
    };
    let mut filtered = make_state(20, 16, Some(blocked_filter));
    let mut filtered_ids = HashSet::new();
    let mut mismatched = terminal_delta(3, "blocked-mismatch", "blocked", 4);
    mismatched
        .preview
        .as_mut()
        .expect("mismatched preview")
        .blocked_binding = Some(BlockedBindingDiagnostic {
        constraint_source: BlockedBindingConstraintSource::UpstreamAccountBinding,
        upstream_account_id: 8,
        upstream_account_label: "Other account".to_string(),
        prompt_cache_key: Some("blocked".to_string()),
        recovery_action: BlockedBindingRecoveryAction::ClearAndResetAffinity,
    });
    assert_eq!(
        filtered
            .apply_deltas(&[mismatched], &mut filtered_ids, 0)
            .expect("reject mismatched blocked binding"),
        WorkingConversationsProjectionUpdate::Unchanged
    );
    let mut matched = terminal_delta(4, "blocked-match", "blocked", 3);
    matched
        .preview
        .as_mut()
        .expect("matched preview")
        .blocked_binding = Some(BlockedBindingDiagnostic {
        constraint_source: BlockedBindingConstraintSource::UpstreamAccountBinding,
        upstream_account_id: 7,
        upstream_account_label: "Matched account".to_string(),
        prompt_cache_key: Some("blocked".to_string()),
        recovery_action: BlockedBindingRecoveryAction::ClearAndResetAffinity,
    });
    assert_eq!(
        filtered
            .apply_deltas(std::slice::from_ref(&matched), &mut filtered_ids, 0)
            .expect("matching blocked binding requires bounded hydration"),
        WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(BTreeSet::from([
            "blocked".to_string()
        ]))
    );
    seed_hydrated(&mut filtered, "blocked", &matched.occurred_at, 1);
    assert_eq!(
        filtered
            .apply_deltas(&[matched], &mut filtered_ids, 0)
            .expect("accept hydrated matching blocked binding"),
        WorkingConversationsProjectionUpdate::Changed
    );
    assert_eq!(filtered.response.conversations.len(), 1);

    let mut recent = make_state(20, 16, None);
    let mut recent_ids = HashSet::new();
    for index in 0..17 {
        let delta = terminal_delta(
            100 + index,
            &format!("recent-{index:02}"),
            "recent",
            17 - index,
        );
        if index == 0 {
            seed_hydrated(&mut recent, "recent", &delta.occurred_at, 1);
        }
        recent
            .apply_deltas(&[delta], &mut recent_ids, 0)
            .expect("apply ordered recent terminal");
    }
    let recent_conversation = &recent.response.conversations[0];
    assert_eq!(recent_conversation.recent_invocations.len(), 16);
    assert_eq!(
        recent_conversation.recent_invocations[0].invoke_id,
        "recent-16"
    );
    assert_eq!(recent_conversation.request_count, 17);
    let mut expired = recent_conversation.clone();
    expired.prompt_cache_key = "expired".to_string();
    expired.last_activity_at = format_utc_iso(
        now - ChronoDuration::minutes(
            SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES + 1,
        ),
    );
    recent.response.conversations.push(expired);
    *recent
        .response
        .total_matched
        .as_mut()
        .expect("tracked total") += 1;
    recent.response.conversations[0].last24h_requests.push(
        PromptCacheConversationRequestPointResponse {
            occurred_at: format_utc_iso(now - ChronoDuration::hours(25)),
            status: "success".to_string(),
            is_success: true,
            outcome: "success".to_string(),
            request_tokens: 1,
            cumulative_tokens: 43,
        },
    );
    assert!(recent.expire(now));
    assert!(
        recent
            .response
            .conversations
            .iter()
            .all(|conversation| conversation.prompt_cache_key != "expired")
    );
    assert_eq!(recent.response.total_matched, Some(1));
    assert!(
        recent.response.conversations[0]
            .last24h_requests
            .iter()
            .all(|point| parse_to_utc_datetime(&point.occurred_at)
                .is_some_and(|occurred_at| occurred_at >= now - ChronoDuration::hours(24)))
    );
}

#[test]
fn typed_runtime_removal_clears_the_matching_prompt_cache_preview() {
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let mut payload = serde_json::json!({ "conversations": [] });
    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.invoke_id = "runtime-removal".to_string();
    record.prompt_cache_key = Some("cache-key".to_string());
    record.status = Some("running".to_string());

    let upsert = PromptCacheTopicDelta::from_record(&record)
        .expect("build compact runtime delta")
        .expect("prompt cache runtime delta");
    let RuntimeMutation::Invocation(removal) =
        RuntimeMutation::invocation(&record, RuntimeMutationKind::RuntimeRemoved)
    else {
        unreachable!("runtime removal must produce an invocation mutation");
    };
    let removal = PromptCacheTopicDelta::from_runtime_mutation(&removal, None)
        .expect("build compact removal delta")
        .expect("prompt cache removal delta");
    let mut applied_terminal_ids = HashSet::new();

    apply_prompt_cache_records_to_payload(
        &topic,
        &mut payload,
        &[upsert],
        &mut applied_terminal_ids,
        0,
    )
    .expect("apply runtime preview");
    assert_eq!(
        payload["conversations"][0]["recentInvocations"]
            .as_array()
            .expect("recent previews")
            .len(),
        1
    );

    assert!(
        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            &[removal],
            &mut applied_terminal_ids,
            0,
        )
        .expect("remove runtime preview")
    );
    assert!(
        payload["conversations"][0]["recentInvocations"]
            .as_array()
            .expect("recent previews")
            .is_empty()
    );
}

#[test]
fn prompt_cache_sticky_projection_uses_sticky_key_without_prompt_outcome() {
    let topic = SubscriptionTopic::PromptCacheStickyWindow {
        account_id: 9,
        selection: AccountStickyKeySelection::Count(20),
    };
    let mut payload = serde_json::json!({ "conversations": [] });
    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.invoke_id = "sticky-terminal".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("prompt-key".to_string());
    record.sticky_key = Some("sticky-key".to_string());
    record.upstream_account_id = Some(9);
    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build compact delta")
        .expect("sticky delta");
    let mut applied_terminal_ids = HashSet::new();

    assert!(
        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            &[delta],
            &mut applied_terminal_ids,
            0,
        )
        .expect("apply sticky delta")
    );
    let conversation = &payload["conversations"][0];
    assert_eq!(conversation["stickyKey"], "sticky-key");
    assert!(conversation["last24hRequests"][0].get("outcome").is_none());
}

#[test]
fn prompt_cache_terminal_dedup_survives_recent_truncation() {
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(1),
    };
    let mut payload = serde_json::json!({ "conversations": [] });
    let mut first = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    first.invoke_id = "first-terminal".to_string();
    first.status = Some("success".to_string());
    first.live_phase = None;
    first.prompt_cache_key = Some("cache-key".to_string());
    let first = PromptCacheTopicDelta::from_record(&first)
        .expect("build first delta")
        .expect("first delta");
    let mut second = dashboard_runtime_topology_live_record("2026-08-08 10:01:00");
    second.invoke_id = "second-terminal".to_string();
    second.status = Some("success".to_string());
    second.live_phase = None;
    second.prompt_cache_key = Some("cache-key".to_string());
    let second = PromptCacheTopicDelta::from_record(&second)
        .expect("build second delta")
        .expect("second delta");
    let mut applied_terminal_ids = HashSet::new();

    apply_prompt_cache_records_to_payload(
        &topic,
        &mut payload,
        &[first.clone(), second],
        &mut applied_terminal_ids,
        0,
    )
    .expect("apply terminal deltas");
    apply_prompt_cache_records_to_payload(
        &topic,
        &mut payload,
        &[first],
        &mut applied_terminal_ids,
        0,
    )
    .expect("replay truncated terminal");

    assert_eq!(payload["conversations"][0]["requestCount"], 2);
    assert_eq!(
        payload["conversations"][0]["recentInvocations"]
            .as_array()
            .expect("recent invocations")
            .len(),
        1
    );
}

#[test]
fn prompt_cache_cold_start_marks_truncated_runtime_terminals_as_applied() {
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(4),
    };
    let records = (0..5)
        .map(|index| {
            let mut record =
                dashboard_runtime_topology_live_record(&format!("2026-08-08 10:0{index}:00"));
            record.invoke_id = format!("cold-start-terminal-{index}");
            record.status = Some("success".to_string());
            record.live_phase = None;
            record.prompt_cache_key = Some("cold-start-key".to_string());
            record.total_tokens = Some(1);
            record.cost = Some(0.1);
            record
        })
        .collect::<Vec<_>>();
    let payload = BuiltSubscriptionTopicPayload::Json(json!({
        "conversations": [{
            "promptCacheKey": "cold-start-key",
            "requestCount": 5,
            "successCount": 5,
            "failureCount": 0,
            "totalTokens": 5,
            "totalCost": 0.5,
            "createdAt": "2026-08-08 10:00:00",
            "lastActivityAt": "2026-08-08 10:04:00",
            "lastTerminalAt": "2026-08-08 10:04:00",
            "recentInvocations": [],
            "last24hRequests": []
        }]
    }));
    let applied_terminal_ids =
        prompt_cache_baseline_runtime_terminal_identities(&topic, &payload, &records);
    assert_eq!(applied_terminal_ids.len(), 5);
    assert!(applied_terminal_ids.contains(&runtime_prompt_cache_overlay_identity(&records[0])));

    let deltas = records
        .iter()
        .map(|record| {
            PromptCacheTopicDelta::from_record(record)
                .expect("build cold-start replay delta")
                .expect("terminal record produces a replay delta")
        })
        .collect::<Vec<_>>();
    let mut replay_applied_terminal_ids = applied_terminal_ids;
    let mut replay_payload = payload.snapshot_payload();
    apply_prompt_cache_records_to_payload(
        &topic,
        &mut replay_payload,
        &deltas,
        &mut replay_applied_terminal_ids,
        0,
    )
    .expect("replay cold-start terminal records");

    assert_eq!(replay_payload["conversations"][0]["requestCount"], 5);
    assert_eq!(replay_payload["conversations"][0]["totalTokens"], 5);
    assert_eq!(
        replay_payload["conversations"][0]["recentInvocations"]
            .as_array()
            .expect("recent invocations")
            .len(),
        4
    );
}

#[test]
fn prompt_cache_baseline_cursor_skips_recovered_terminal_totals() {
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let mut payload = serde_json::json!({ "conversations": [] });
    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.id = 7;
    record.invoke_id = "recovered-terminal".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("cache-key".to_string());
    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build recovered delta")
        .expect("recovered delta");
    let delta_identity = delta.identity.clone();
    let mut applied_terminal_ids = HashSet::new();

    apply_prompt_cache_records_to_payload(
        &topic,
        &mut payload,
        &[delta],
        &mut applied_terminal_ids,
        7,
    )
    .expect("apply recovered delta");

    assert_eq!(payload["conversations"][0]["requestCount"], 0);
    assert!(applied_terminal_ids.contains(&delta_identity));
    assert_eq!(
        payload["conversations"][0]["recentInvocations"]
            .as_array()
            .expect("recent invocations")
            .len(),
        1
    );
}
