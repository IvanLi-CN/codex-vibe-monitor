fn working_make_state(
    now: DateTime<Utc>,
    page_size: i64,
    recent_invocation_limit: i64,
    blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
) -> DashboardWorkingConversationsMaterializerState {
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
}

fn working_local_time(now: DateTime<Utc>, offset_seconds: i64) -> String {
    format_naive(
        (now - ChronoDuration::seconds(offset_seconds))
            .with_timezone(&Shanghai)
            .naive_local(),
    )
}

fn working_terminal_delta(
    now: DateTime<Utc>,
    id: i64,
    invoke_id: &str,
    prompt_cache_key: &str,
    offset_seconds: i64,
) -> PromptCacheTopicDelta {
    let mut record =
        dashboard_runtime_topology_live_record(&working_local_time(now, offset_seconds));
    record.id = id;
    record.invoke_id = invoke_id.to_string();
    record.prompt_cache_key = Some(prompt_cache_key.to_string());
    record.upstream_account_id = None;
    record.upstream_account_name = None;
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.total_tokens = Some(42);
    record.cost = Some(0.25);
    PromptCacheTopicDelta::from_record(&record)
        .expect("build working terminal delta")
        .expect("working terminal delta")
}

fn seed_working_hydrated(
    state: &mut DashboardWorkingConversationsMaterializerState,
    prompt_cache_key: &str,
    occurred_at: &str,
    total_matched: i64,
) {
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
}

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
fn working_conversations_projection_preserves_stateful_runtime_contract() {
    let now = Utc::now();

    let (mut projection, mut applied_terminal_ids) = working_projection_running_contract(now);
    assert_runtime_preview_removal_contract(now);
    assert_old_in_flight_contract(now);
    assert_terminal_replacement_contract(now, &mut projection, &mut applied_terminal_ids);
    assert_account_history_contract(now);
    assert_binding_contract(now, &mut projection, &mut applied_terminal_ids);
    assert_blocked_binding_contract(now);
    assert_recent_expiry_contract(now);
}

fn working_projection_running_contract(
    now: DateTime<Utc>,
) -> (
    DashboardWorkingConversationsMaterializerState,
    HashSet<String>,
) {
    let mut projection = working_make_state(now, 1, 16, None);
    let baseline_window = (
        projection.response.range_start.clone(),
        projection.response.range_end.clone(),
        projection.response.snapshot_at.clone(),
    );
    let mut applied_terminal_ids = HashSet::new();
    let mut running = dashboard_runtime_topology_live_record(&working_local_time(now, 30));
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
    seed_working_hydrated(&mut projection, "alpha", &running.occurred_at, 1);
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

    (projection, applied_terminal_ids)
}

fn assert_runtime_preview_removal_contract(now: DateTime<Utc>) {
    let mut transient = working_make_state(now, 20, 16, None);
    let mut transient_ids = HashSet::new();
    let mut transient_record = dashboard_runtime_topology_live_record(&working_local_time(now, 25));
    transient_record.id = 2;
    transient_record.invoke_id = "runtime-removed".to_string();
    transient_record.prompt_cache_key = Some("transient".to_string());
    let transient_upsert = PromptCacheTopicDelta::from_record(&transient_record)
        .expect("build transient runtime delta")
        .expect("transient runtime delta");
    seed_working_hydrated(
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
}

fn assert_old_in_flight_contract(now: DateTime<Utc>) {
    let mut old_in_flight = working_make_state(now, 20, 16, None);
    let mut old_in_flight_ids = HashSet::new();
    let mut old_in_flight_record =
        dashboard_runtime_topology_live_record(&working_local_time(now, 16 * 60));
    old_in_flight_record.id = 3;
    old_in_flight_record.invoke_id = "long-running".to_string();
    old_in_flight_record.prompt_cache_key = Some("long-running".to_string());
    let old_in_flight_delta = PromptCacheTopicDelta::from_record(&old_in_flight_record)
        .expect("build old in-flight delta")
        .expect("old in-flight delta");
    seed_working_hydrated(
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
}

fn assert_terminal_replacement_contract(
    now: DateTime<Utc>,
    projection: &mut DashboardWorkingConversationsMaterializerState,
    applied_terminal_ids: &mut HashSet<String>,
) {
    let terminal = working_terminal_delta(now, 1, "runtime-to-terminal", "alpha", 30);
    assert_eq!(
        projection
            .apply_deltas(std::slice::from_ref(&terminal), applied_terminal_ids, 0)
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
    assert!(alpha.last_in_flight_at.is_none());
    assert_eq!(alpha.last24h_requests.len(), 1);
    assert_eq!(alpha.upstream_accounts[0].upstream_account_id, None);
    let same_second_terminal = working_terminal_delta(now, 4, "same-second-terminal", "alpha", 30);
    assert_eq!(
        projection
            .apply_deltas(&[same_second_terminal], applied_terminal_ids, 0)
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
            .apply_deltas(&[terminal], applied_terminal_ids, 0)
            .expect("deduplicate terminal replay"),
        WorkingConversationsProjectionUpdate::Unchanged
    );
    assert_eq!(projection.response.conversations[0].request_count, 2);

    let mut alpha_runtime_record =
        dashboard_runtime_topology_live_record(&working_local_time(now, 10));
    alpha_runtime_record.id = 3;
    alpha_runtime_record.invoke_id = "alpha-runtime-preview".to_string();
    alpha_runtime_record.prompt_cache_key = Some("alpha".to_string());
    let alpha_runtime_preview = PromptCacheTopicDelta::from_record(&alpha_runtime_record)
        .expect("build alpha runtime preview")
        .expect("alpha runtime preview");
    assert_eq!(
        projection
            .apply_deltas(&[alpha_runtime_preview], applied_terminal_ids, 0)
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
            .apply_deltas(&[alpha_runtime_removal], applied_terminal_ids, 0)
            .expect("remove alpha runtime preview"),
        WorkingConversationsProjectionUpdate::Changed
    );
    let alpha = &projection.response.conversations[0];
    assert_eq!(alpha.last_in_flight_at, None);
    assert_eq!(alpha.last_activity_at, terminal_activity_at);
}

fn assert_account_history_contract(now: DateTime<Utc>) {
    let mut account_history = working_make_state(now, 20, 16, None);
    let mut account_history_ids = HashSet::new();
    let account_history_delta =
        working_terminal_delta(now, 200, "account-history-base", "account-history", 3);
    seed_working_hydrated(
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
        working_terminal_delta(now, 201, "omitted-account-live", "account-history", 2);
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
}

fn assert_binding_contract(
    now: DateTime<Utc>,
    projection: &mut DashboardWorkingConversationsMaterializerState,
    applied_terminal_ids: &mut HashSet<String>,
) {
    let beta = working_terminal_delta(now, 2, "newer-terminal", "beta", 5);
    seed_working_hydrated(projection, "beta", &beta.occurred_at, 2);
    assert_eq!(
        projection
            .apply_deltas(&[beta], applied_terminal_ids, 0)
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
}

fn assert_blocked_binding_contract(now: DateTime<Utc>) {
    let blocked_filter = PromptCacheConversationBlockedBindingFilter {
        upstream_account_id: Some(7),
        constraint_source: Some(BlockedBindingConstraintSource::UpstreamAccountBinding),
    };
    let mut filtered = working_make_state(now, 20, 16, Some(blocked_filter));
    let mut filtered_ids = HashSet::new();
    let mut mismatched = working_terminal_delta(now, 3, "blocked-mismatch", "blocked", 4);
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
    let mut matched = working_terminal_delta(now, 4, "blocked-match", "blocked", 3);
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
    seed_working_hydrated(&mut filtered, "blocked", &matched.occurred_at, 1);
    assert_eq!(
        filtered
            .apply_deltas(&[matched], &mut filtered_ids, 0)
            .expect("accept hydrated matching blocked binding"),
        WorkingConversationsProjectionUpdate::Changed
    );
    assert_eq!(filtered.response.conversations.len(), 1);
}

fn assert_recent_expiry_contract(now: DateTime<Utc>) {
    let mut recent = working_make_state(now, 20, 16, None);
    let mut recent_ids = HashSet::new();
    for index in 0..17 {
        let delta = working_terminal_delta(
            now,
            100 + index,
            &format!("recent-{index:02}"),
            "recent",
            17 - index,
        );
        if index == 0 {
            seed_working_hydrated(&mut recent, "recent", &delta.occurred_at, 1);
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
    assert!(applied_terminal_ids.is_empty());
    assert_eq!(
        payload["conversations"][0]["recentInvocations"]
            .as_array()
            .expect("recent invocations")
            .len(),
        1
    );
}

#[test]
fn prompt_cache_baseline_does_not_replay_an_identity_already_in_payload() {
    let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    record.id = 0;
    record.invoke_id = "persisted-during-baseline".to_string();
    record.status = Some("success".to_string());
    record.live_phase = None;
    record.prompt_cache_key = Some("cache-key".to_string());
    let delta = PromptCacheTopicDelta::from_record(&record)
        .expect("build concurrent delta")
        .expect("prompt cache delta");

    assert!(prompt_cache_delta_needs_replay(&delta, &HashSet::new()));
    assert!(!prompt_cache_delta_needs_replay(
        &delta,
        &HashSet::from([delta.identity.clone()]),
    ));
}

#[test]
fn prompt_cache_projection_keeps_latest_activity_for_out_of_order_records() {
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    let mut payload = serde_json::json!({ "conversations": [] });
    let mut newer = dashboard_runtime_topology_live_record("2026-08-08 10:01:00");
    newer.invoke_id = "newer".to_string();
    newer.prompt_cache_key = Some("cache-key".to_string());
    newer.status = Some("success".to_string());
    newer.live_phase = None;
    newer.upstream_account_id = Some(9);
    let newer = PromptCacheTopicDelta::from_record(&newer)
        .expect("build newer delta")
        .expect("newer delta");
    let mut older = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
    older.invoke_id = "older".to_string();
    older.prompt_cache_key = Some("cache-key".to_string());
    older.status = Some("success".to_string());
    older.live_phase = None;
    older.upstream_account_id = Some(9);
    let older = PromptCacheTopicDelta::from_record(&older)
        .expect("build older delta")
        .expect("older delta");
    let mut applied_terminal_ids = HashSet::new();

    apply_prompt_cache_records_to_payload(
        &topic,
        &mut payload,
        &[newer, older],
        &mut applied_terminal_ids,
        0,
    )
    .expect("apply out-of-order deltas");

    assert_eq!(
        payload["conversations"][0]["lastActivityAt"],
        "2026-08-08T02:01:00Z"
    );
    assert_eq!(
        payload["conversations"][0]["createdAt"],
        "2026-08-08T02:00:00Z"
    );
    assert_eq!(
        payload["conversations"][0]["lastTerminalAt"],
        "2026-08-08T02:01:00Z"
    );
    assert_eq!(
        payload["conversations"][0]["upstreamAccounts"][0]["lastActivityAt"],
        "2026-08-08T02:01:00Z"
    );
}

#[test]
fn prompt_cache_binding_patch_updates_only_the_selected_conversation() {
    let mut payload = serde_json::json!({
        "conversations": [
            { "promptCacheKey": "selected", "hasEncryptedSessionOwner": false },
            { "promptCacheKey": "other", "hasEncryptedSessionOwner": false }
        ]
    });
    let binding = serde_json::json!({
        "bindingKind": "account",
        "groupName": "primary",
        "upstreamAccountId": 9,
        "upstreamAccountName": "Account 9",
        "hasEncryptedSessionOwner": true,
        "encryptedOwnerAccountId": 8,
        "encryptedOwnerAccountName": "Account 8",
        "encryptedOwnerGroupName": "owners"
    });

    assert_eq!(
        patch_prompt_cache_binding_payload(&mut payload, "selected", &binding),
        Some(true)
    );
    assert_eq!(
        payload["conversations"][0]["manualBinding"]["upstreamAccountId"],
        9
    );
    assert_eq!(
        payload["conversations"][0]["hasEncryptedSessionOwner"],
        true
    );
    assert_eq!(
        payload["conversations"][1]["hasEncryptedSessionOwner"],
        false
    );
}

#[test]
fn subscription_event_envelope_serializes_camel_case_fields() {
    let payload = SubscriptionEventEnvelope::Snapshot {
        topic: SubscriptionTopicDescriptor {
            topic: "app.version".to_string(),
            params: BTreeMap::new(),
        },
        topic_key: "topic-key".to_string(),
        schema_epoch: "app.version/v1".to_string(),
        cursor: 7,
        payload: json!({
            "backend": "0.2.0-dev",
            "frontend": "0.2.0-dev",
        }),
    };

    let encoded = serde_json::to_value(payload).expect("serialize envelope");

    assert_eq!(encoded.get("topicKey"), Some(&json!("topic-key")));
    assert_eq!(encoded.get("schemaEpoch"), Some(&json!("app.version/v1")));
    assert!(encoded.get("topic_key").is_none());
    assert!(encoded.get("schema_epoch").is_none());
}

#[test]
fn serialized_topic_frame_preserves_snapshot_replay_and_live_wire_kinds() {
    let topic = summary_topic();
    let frame = serialize_topic_frame(
        topic.descriptor(),
        topic.cache_key().expect("topic key"),
        topic.schema_epoch(),
        7,
        serde_json::to_vec(&json!({ "total": 3 })).expect("payload"),
    )
    .expect("serialized frame");

    for (kind, expected) in [
        (TopicFrameKind::Snapshot, "snapshot"),
        (TopicFrameKind::Replay, "replay"),
        (TopicFrameKind::Live, "live"),
    ] {
        let chunks = frame.event_chunks(kind);
        let wire = chunks.concat();
        let envelope: Value =
            serde_json::from_slice(&wire[6..wire.len() - 2]).expect("wire envelope");
        assert_eq!(envelope["type"], expected);
        assert_eq!(envelope["cursor"], 7);
        assert_eq!(envelope["payload"], json!({ "total": 3 }));
    }
}

#[test]
fn decode_resume_query_accepts_legacy_topic_key_format() {
    let descriptor = summary_topic().descriptor();
    let topic_key = summary_topic().cache_key().expect("topic key");
    let raw = serde_json::to_string(&vec![json!({
        "topicKey": topic_key.clone(),
        "cursor": 4,
        "schemaEpoch": "stats.summary.current/v1",
    })])
    .expect("encode resume query");

    let decoded = decode_resume_query(Some(&raw), &[descriptor]).expect("decode resume");

    assert_eq!(
        decoded,
        vec![SubscriptionResumeCursor {
            topic_key,
            cursor: 4,
            schema_epoch: "stats.summary.current/v1".to_string(),
        }]
    );
}

#[test]
fn decode_resume_query_accepts_compact_topic_index_format() {
    let descriptor = summary_topic().descriptor();
    let topic_key = summary_topic().cache_key().expect("topic key");
    let raw = serde_json::to_string(&vec![json!({
        "topicIndex": 0,
        "cursor": 4,
        "schemaEpoch": "stats.summary.current/v1",
    })])
    .expect("encode resume query");

    let decoded = decode_resume_query(Some(&raw), &[descriptor]).expect("decode resume");

    assert_eq!(
        decoded,
        vec![SubscriptionResumeCursor {
            topic_key,
            cursor: 4,
            schema_epoch: "stats.summary.current/v1".to_string(),
        }]
    );
}

#[test]
fn decode_resume_query_rejects_out_of_range_compact_topic_index() {
    let descriptor = summary_topic().descriptor();
    let raw = serde_json::to_string(&vec![json!({
        "topicIndex": 1,
        "cursor": 4,
        "schemaEpoch": "stats.summary.current/v1",
    })])
    .expect("encode resume query");

    let error = decode_resume_query(Some(&raw), &[descriptor]).expect_err("resume should reject");

    match error {
        ApiError::BadRequest(err) => {
            assert!(format!("{err}").contains("resume topicIndex out of range: 1"));
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[tokio::test]
async fn replay_returns_gap_when_cursor_is_within_window() {
    let hub = SubscriptionHub::new();
    let topic = summary_topic();
    let topic_key = topic.cache_key().expect("topic key");
    let schema_epoch = topic.schema_epoch();
    let cached = seeded_cached_topic(topic, &[1, 2, 3, 4], Utc::now());
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);

    let replay = hub
        .replay_events_for_resume(
            &topic_key,
            schema_epoch.clone(),
            Some(&SubscriptionResumeCursor {
                topic_key: topic_key.clone(),
                cursor: 2,
                schema_epoch,
            }),
        )
        .await
        .expect("replay should be eligible")
        .expect("replay gap should exist");

    assert_eq!(
        replay
            .iter()
            .map(|event| event.frame.cursor)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
}

#[tokio::test]
async fn replay_rejects_schema_epoch_mismatch() {
    let hub = SubscriptionHub::new();
    let topic = summary_topic();
    let topic_key = topic.cache_key().expect("topic key");
    let cached = seeded_cached_topic(topic, &[1, 2], Utc::now());
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);

    let result = hub
        .replay_events_for_resume(
            &topic_key,
            "stats.summary.current/v1".to_string(),
            Some(&SubscriptionResumeCursor {
                topic_key: topic_key.clone(),
                cursor: 1,
                schema_epoch: "stats.summary.current/v0".to_string(),
            }),
        )
        .await;

    assert!(matches!(result, Err(ReplayMissReason::SchemaEpochMismatch)));
}

#[tokio::test]
async fn replay_rejects_window_miss_when_cursor_is_older_than_front() {
    let hub = SubscriptionHub::new();
    let topic = summary_topic();
    let topic_key = topic.cache_key().expect("topic key");
    let schema_epoch = topic.schema_epoch();
    let cached = seeded_cached_topic(topic, &[10, 11, 12], Utc::now());
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);

    let result = hub
        .replay_events_for_resume(
            &topic_key,
            schema_epoch.clone(),
            Some(&SubscriptionResumeCursor {
                topic_key: topic_key.clone(),
                cursor: 8,
                schema_epoch,
            }),
        )
        .await;

    assert!(matches!(result, Err(ReplayMissReason::GapWindowMiss)));
}

#[tokio::test]
async fn replay_rejects_gaps_that_exceed_event_budget() {
    let hub = SubscriptionHub::new();
    let topic = summary_topic();
    let topic_key = topic.cache_key().expect("topic key");
    let schema_epoch = topic.schema_epoch();
    let cursors = (1..=(SUBSCRIPTION_REPLAY_MAX_GAP_EVENTS as u64 + 2)).collect::<Vec<_>>();
    let cached = seeded_cached_topic(topic, &cursors, Utc::now());
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);

    let result = hub
        .replay_events_for_resume(
            &topic_key,
            schema_epoch.clone(),
            Some(&SubscriptionResumeCursor {
                topic_key: topic_key.clone(),
                cursor: 1,
                schema_epoch,
            }),
        )
        .await;

    assert!(matches!(
        result,
        Err(ReplayMissReason::GapEventBudgetExceeded)
    ));
}

#[tokio::test]
async fn prepare_connection_reports_snapshot_without_resume() {
    let state =
        crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap()).await;
    let hub = SubscriptionHub::new();
    let topic = summary_topic();
    let descriptor = topic.descriptor();
    let topic_key = topic.cache_key().expect("topic key");
    let cached = seeded_cached_topic(topic, &[1, 2, 3], Utc::now());
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);

    let prepared = hub
        .prepare_connection(state, vec![descriptor], Vec::new())
        .await
        .expect("prepare connection");

    assert_eq!(prepared.initial.len(), 1);
    assert_eq!(
        prepared.outcomes,
        vec![TopicInitOutcome {
            topic_key,
            disposition: TopicInitDisposition::SnapshotNoResume,
            replay_event_count: 0,
            replay_bytes: 0,
            cursor: 3,
            miss_reason: None,
        }]
    );
}

#[tokio::test]
async fn prepare_connection_reports_replay_hit_and_resume_caught_up() {
    let state =
        crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap()).await;
    let hub = SubscriptionHub::new();
    let topic = summary_topic();
    let descriptor = topic.descriptor();
    let topic_key = topic.cache_key().expect("topic key");
    let schema_epoch = topic.schema_epoch();
    let cached = seeded_cached_topic(topic, &[1, 2, 3, 4], Utc::now());
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);

    let replay_hit = hub
        .prepare_connection(
            state.clone(),
            vec![descriptor.clone()],
            vec![SubscriptionResumeCursor {
                topic_key: topic_key.clone(),
                cursor: 2,
                schema_epoch: schema_epoch.clone(),
            }],
        )
        .await
        .expect("prepare connection");
    assert_eq!(replay_hit.initial.len(), 2);
    assert_eq!(
        replay_hit.outcomes[0],
        TopicInitOutcome {
            topic_key: topic_key.clone(),
            disposition: TopicInitDisposition::ReplayHit,
            replay_event_count: 2,
            replay_bytes: 64,
            cursor: 4,
            miss_reason: None,
        }
    );

    let caught_up = hub
        .prepare_connection(
            state,
            vec![descriptor],
            vec![SubscriptionResumeCursor {
                topic_key: topic_key.clone(),
                cursor: 4,
                schema_epoch,
            }],
        )
        .await
        .expect("prepare connection");
    assert!(caught_up.initial.is_empty());
    assert_eq!(
        caught_up.outcomes[0],
        TopicInitOutcome {
            topic_key,
            disposition: TopicInitDisposition::ResumeCaughtUp,
            replay_event_count: 0,
            replay_bytes: 0,
            cursor: 4,
            miss_reason: None,
        }
    );
}

#[tokio::test]
async fn prepare_connection_reports_snapshot_resume_miss() {
    let state =
        crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap()).await;
    let hub = SubscriptionHub::new();
    let topic = summary_topic();
    let descriptor = topic.descriptor();
    let topic_key = topic.cache_key().expect("topic key");
    let cached = seeded_cached_topic(topic, &[1, 2, 3], Utc::now());
    hub.state
        .lock()
        .await
        .topics
        .insert(topic_key.clone(), cached);

    let prepared = hub
        .prepare_connection(
            state,
            vec![descriptor],
            vec![SubscriptionResumeCursor {
                topic_key: topic_key.clone(),
                cursor: 2,
                schema_epoch: "stats.summary.current/v0".to_string(),
            }],
        )
        .await
        .expect("prepare connection");

    assert_eq!(prepared.initial.len(), 1);
    assert_eq!(
        prepared.outcomes,
        vec![TopicInitOutcome {
            topic_key,
            disposition: TopicInitDisposition::SnapshotResumeMiss,
            replay_event_count: 0,
            replay_bytes: 0,
            cursor: 3,
            miss_reason: Some("schema_epoch_mismatch"),
        }]
    );
}

#[tokio::test]
async fn invalidate_dashboard_activity_snapshot_cache_only_removes_selected_entry() {
    let cache = Arc::new(Mutex::new(DashboardActivitySnapshotCacheState::default()));
    let selection_a = DashboardActivitySnapshotSelection {
        range: "today".to_string(),
        range_anchor: "2026-07-20".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        source_scope: "all".to_string(),
        recent_limit: 4,
        include_accounts: true,
        include_recent: true,
    };
    let selection_b = DashboardActivitySnapshotSelection {
        range: "7d".to_string(),
        range_anchor: "rolling".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        source_scope: "all".to_string(),
        recent_limit: 8,
        include_accounts: true,
        include_recent: true,
    };
    let (signal_a, mut rx_a) = watch::channel(false);
    let (signal_b, rx_b) = watch::channel(false);

    {
        let mut guard = cache.lock().await;
        guard.entries.insert(
            selection_a.clone(),
            DashboardActivitySnapshotCacheEntry {
                cached_at: Instant::now(),
                last_reconcile_attempted_at: Instant::now(),
                last_reconcile_failed: false,
                baseline_snapshot_cursor: 0,
                expiry_covered_until: None,
                expiry_terminal_deltas: VecDeque::new(),
                expiry_delta_estimated_bytes: 0,
                response: DashboardActivitySnapshot::test_stub("today"),
            },
        );
        guard.entries.insert(
            selection_b.clone(),
            DashboardActivitySnapshotCacheEntry {
                cached_at: Instant::now(),
                last_reconcile_attempted_at: Instant::now(),
                last_reconcile_failed: false,
                baseline_snapshot_cursor: 0,
                expiry_covered_until: None,
                expiry_terminal_deltas: VecDeque::new(),
                expiry_delta_estimated_bytes: 0,
                response: DashboardActivitySnapshot::test_stub("7d"),
            },
        );
        guard.in_flight.insert(
            selection_a.clone(),
            DashboardActivitySnapshotInFlight {
                signal: signal_a,
                waiter_count: 2,
                baseline_cursor: None,
                routing_rules_generation: 0,
            },
        );
        guard.in_flight.insert(
            selection_b.clone(),
            DashboardActivitySnapshotInFlight {
                signal: signal_b,
                waiter_count: 3,
                baseline_cursor: None,
                routing_rules_generation: 0,
            },
        );
    }

    invalidate_dashboard_activity_snapshot_cache(
        cache.as_ref(),
        &selection_a,
        "scheduled_terminal_refresh",
    )
    .await;

    rx_a.changed()
        .await
        .expect("selected in-flight should be signaled");
    assert!(rx_a.borrow().to_owned());
    assert!(!*rx_b.borrow());

    let guard = cache.lock().await;
    assert!(!guard.entries.contains_key(&selection_a));
    assert!(guard.entries.contains_key(&selection_b));
    assert!(!guard.in_flight.contains_key(&selection_a));
    assert!(guard.in_flight.contains_key(&selection_b));
}

#[tokio::test]
async fn routing_rule_invalidation_keeps_summary_only_entries_and_advances_fence() {
    let cache = Arc::new(Mutex::new(DashboardActivitySnapshotCacheState::default()));
    let account_selection = DashboardActivitySnapshotSelection {
        range: "today".to_string(),
        range_anchor: "2026-07-20".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        source_scope: "all".to_string(),
        recent_limit: 4,
        include_accounts: true,
        include_recent: true,
    };
    let summary_selection = DashboardActivitySnapshotSelection {
        include_accounts: false,
        include_recent: false,
        ..account_selection.clone()
    };
    let (signal, mut receiver) = watch::channel(false);
    {
        let mut guard = cache.lock().await;
        guard.entries.insert(
            account_selection.clone(),
            DashboardActivitySnapshotCacheEntry {
                cached_at: Instant::now(),
                last_reconcile_attempted_at: Instant::now(),
                last_reconcile_failed: false,
                baseline_snapshot_cursor: 0,
                expiry_covered_until: None,
                expiry_terminal_deltas: VecDeque::new(),
                expiry_delta_estimated_bytes: 0,
                response: DashboardActivitySnapshot::test_stub("today"),
            },
        );
        guard.entries.insert(
            summary_selection.clone(),
            DashboardActivitySnapshotCacheEntry {
                cached_at: Instant::now(),
                last_reconcile_attempted_at: Instant::now(),
                last_reconcile_failed: false,
                baseline_snapshot_cursor: 0,
                expiry_covered_until: None,
                expiry_terminal_deltas: VecDeque::new(),
                expiry_delta_estimated_bytes: 0,
                response: DashboardActivitySnapshot::test_stub("today"),
            },
        );
        guard.in_flight.insert(
            account_selection,
            DashboardActivitySnapshotInFlight {
                signal,
                waiter_count: 0,
                baseline_cursor: None,
                routing_rules_generation: 0,
            },
        );
    }

    invalidate_dashboard_activity_snapshots_with_accounts(
        cache.as_ref(),
        "account_effective_routing_rules_changed",
    )
    .await;

    receiver
        .changed()
        .await
        .expect("routing flight should be signaled");
    let guard = cache.lock().await;
    assert_eq!(guard.routing_rules_generation, 1);
    assert_eq!(guard.entries.len(), 1);
    assert!(guard.entries.contains_key(&summary_selection));
    assert!(guard.in_flight.is_empty());
}

#[tokio::test]
async fn dashboard_activity_live_overlay_clears_stale_optional_latency_fields() {
    let state =
        crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap()).await;
    let now = Utc::now();
    let mut payload = json!({
        "rangeStart": now.to_rfc3339(),
        "rangeEnd": (now + ChronoDuration::minutes(5)).to_rfc3339(),
        "summary": {
            "stats": {
                "inProgressConversationCount": 3,
                "inProgressRetryConversationCount": 2,
                "inProgressPhaseCounts": {}
            },
            "modelPerformance": {
                "available": false
            },
            "tokensPerMinute": 123.0,
            "spendRate": 4.56,
            "currentFirstResponseByteTotalAvgMs": 789.0,
            "currentFirstTokenAvgMs": 987.0,
            "currentAvgTotalMs": 456.0,
            "currentAvgResponseMs": 135.0
        },
        "accounts": [{
            "accountKey": "upstream:42",
            "upstreamAccountId": 42,
            "requestCount": 9,
            "tokensPerMinute": 33.0,
            "spendRate": 1.23,
            "currentFirstResponseByteTotalAvgMs": 654.0,
            "currentFirstTokenAvgMs": 456.0,
            "currentAvgTotalMs": 321.0,
            "currentAvgResponseMs": 246.0,
            "recentInvocations": []
        }]
    });
    let live = DashboardActivityLiveSnapshot {
        revision: 7,
        generated_at: now.to_rfc3339(),
        in_progress_invocation_count: 0,
        in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
        retry_invocation_count: 0,
        in_progress_wait_sum_ms: 0.0,
        in_progress_wait_sample_count: 0,
        network_live_bucket: None,
        network_realtime_rate: None,
        accounts: Vec::new(),
    };

    let applied =
        apply_dashboard_activity_live_overlay_to_payload(state.as_ref(), &mut payload, &live)
            .expect("apply dashboard activity live overlay");

    assert!(applied);
    assert_overlay_clears_stale_latency_fields(&payload);
}

fn assert_overlay_clears_stale_latency_fields(payload: &Value) {
    let summary = payload
        .get("summary")
        .and_then(Value::as_object)
        .expect("summary object");
    for field in [
        "currentFirstResponseByteTotalAvgMs",
        "currentFirstTokenAvgMs",
        "currentAvgTotalMs",
        "currentAvgResponseMs",
    ] {
        assert_eq!(
            summary.get(field),
            None,
            "summary stale field should be removed"
        );
    }
    let account = payload
        .get("accounts")
        .and_then(Value::as_array)
        .and_then(|accounts| accounts.first())
        .and_then(Value::as_object)
        .expect("account object");
    for field in [
        "currentFirstResponseByteTotalAvgMs",
        "currentFirstTokenAvgMs",
        "currentAvgTotalMs",
        "currentAvgResponseMs",
    ] {
        assert_eq!(
            account.get(field),
            None,
            "account stale field should be removed"
        );
    }
}

fn phase_summary_test_state(key: &str) -> DashboardWorkingConversationsMaterializerState {
    let now = Utc::now();
    DashboardWorkingConversationsMaterializerState::new(
        PromptCacheConversationsResponse {
            range_start: format_utc_iso(now - ChronoDuration::minutes(5)),
            range_end: format_utc_iso(now),
            snapshot_at: None,
            selection_mode: PromptCacheConversationSelectionMode::ActivityWindow,
            selected_limit: None,
            selected_activity_hours: None,
            selected_activity_minutes: Some(5),
            implicit_filter: PromptCacheConversationImplicitFilter {
                kind: None,
                filtered_count: 0,
            },
            total_matched: Some(1),
            has_more: false,
            next_cursor: None,
            conversations: vec![PromptCacheConversationResponse {
                prompt_cache_key: key.to_string(),
                request_count: 0,
                total_tokens: 0,
                total_cost: 0.0,
                created_at: format_utc_iso(now),
                last_activity_at: format_utc_iso(now),
                last_terminal_at: None,
                last_in_flight_at: None,
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
            }],
        },
        20,
        16,
        None,
    )
}

fn phase_summary_test_delta(
    identity: &str,
    key: Option<&str>,
    status: &str,
    removed: bool,
) -> PromptCacheTopicDelta {
    PromptCacheTopicDelta {
        row_id: 0,
        identity: identity.to_string(),
        invoke_id: identity.to_string(),
        prompt_cache_key: key.map(str::to_string),
        sticky_key: None,
        occurred_at: "2026-09-03T10:00:00Z".to_string(),
        is_runtime_removed: removed,
        status: status.to_string(),
        is_terminal: status == "success",
        is_success: status == "success",
        request_tokens: 0,
        cost: 0.0,
        upstream_account_id: None,
        upstream_account_name: None,
        preview: None,
    }
}

#[test]
fn working_conversation_phase_summary_covers_all_in_flight_identities() {
    let mut state = phase_summary_test_state("phase-summary");
    state.install_in_flight_phase_records((0..20).map(|index| PromptCacheInFlightPhaseRecord {
        identity: format!("identity-{index}"),
        prompt_cache_key: "phase-summary".to_string(),
        phase: Some(if index == 0 { "queued" } else { "requesting" }.to_string()),
    }));
    assert_eq!(
        state.response.conversations[0]
            .in_flight_phase_counts
            .queued,
        1
    );
    assert_eq!(
        state.response.conversations[0]
            .in_flight_phase_counts
            .requesting,
        19
    );
}

#[test]
fn working_conversation_phase_summary_ignores_recent_preview_limit() {
    let mut state = phase_summary_test_state("phase-summary");
    state.recent_invocation_limit = 1;
    state.install_in_flight_phase_records((0..4).map(|index| PromptCacheInFlightPhaseRecord {
        identity: format!("identity-{index}"),
        prompt_cache_key: "phase-summary".to_string(),
        phase: Some("requesting".to_string()),
    }));
    assert_eq!(
        state.response.conversations[0]
            .in_flight_phase_counts
            .requesting,
        4
    );
}

#[test]
fn working_conversation_phase_summary_updates_on_runtime_transition() {
    let mut state = phase_summary_test_state("phase-summary");
    let mut runtime = dashboard_runtime_topology_live_record("2026-09-03T10:00:00Z");
    runtime.invoke_id = "identity".to_string();
    runtime.prompt_cache_key = Some("phase-summary".to_string());
    runtime.status = Some("running".to_string());
    runtime.live_phase = Some("responding".to_string());
    runtime.first_token_ms = Some(1.0);
    let delta = PromptCacheTopicDelta::from_record(&runtime)
        .expect("build phase transition delta")
        .expect("phase transition delta");
    state.install_in_flight_phase_records([PromptCacheInFlightPhaseRecord {
        identity: delta.identity.clone(),
        prompt_cache_key: "phase-summary".to_string(),
        phase: Some("requesting".to_string()),
    }]);
    let _changed = state.apply_in_flight_phase_delta(&delta);
    assert_eq!(
        state.response.conversations[0]
            .in_flight_phase_counts
            .requesting,
        0
    );
    assert_eq!(
        state.response.conversations[0]
            .in_flight_phase_counts
            .responding,
        1
    );
}

#[test]
fn working_conversation_phase_summary_retracts_terminal_and_runtime_removal() {
    let mut state = phase_summary_test_state("phase-summary");
    state.install_in_flight_phase_records([PromptCacheInFlightPhaseRecord {
        identity: "identity".to_string(),
        prompt_cache_key: "phase-summary".to_string(),
        phase: Some("responding".to_string()),
    }]);
    let terminal = phase_summary_test_delta("identity", Some("phase-summary"), "success", false);
    assert!(state.apply_in_flight_phase_delta(&terminal));
    assert_eq!(
        state.response.conversations[0].in_flight_phase_counts,
        InvocationPhaseCountsResponse::default()
    );
}

#[test]
fn working_conversation_phase_summary_recovery_retains_last_good() {
    let mut state = phase_summary_test_state("phase-summary");
    state.install_in_flight_phase_records([PromptCacheInFlightPhaseRecord {
        identity: "identity".to_string(),
        prompt_cache_key: "phase-summary".to_string(),
        phase: Some("requesting".to_string()),
    }]);
    let unknown_removal = phase_summary_test_delta("unknown", None, "unknown", true);
    assert!(!state.apply_in_flight_phase_delta(&unknown_removal));
    assert_eq!(
        state.response.conversations[0]
            .in_flight_phase_counts
            .requesting,
        1
    );
}
