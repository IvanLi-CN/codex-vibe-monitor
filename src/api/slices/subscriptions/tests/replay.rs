use super::*;
use serde_json::json;
#[test]
fn prune_replay_window_enforces_event_cap() {
    let mut events = VecDeque::new();
    let mut total_bytes = 0usize;
    let topic = summary_topic();
    let descriptor = topic.descriptor();
    let schema_epoch = topic.schema_epoch();
    for index in 0..(SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC + 8) {
        events.push_back(test_replay_event(
            &descriptor,
            &schema_epoch,
            index as u64 + 1,
            Utc::now(),
            32,
        ));
        total_bytes += 32;
    }

    prune_replay_window(&mut events, &mut total_bytes);

    assert!(events.len() <= SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC);
}

#[test]
fn serialized_topic_frame_reuses_shared_chunks_for_each_delivery_kind() {
    let topic = summary_topic();
    let descriptor = topic.descriptor();
    let payload = serde_json::to_vec(&json!({ "value": 42 })).expect("payload");
    let frame = serialize_topic_frame(
        descriptor,
        topic.cache_key().expect("topic key"),
        topic.schema_epoch(),
        7,
        payload,
    )
    .expect("serialized frame");

    for (kind, expected_kind) in [
        (TopicFrameKind::Snapshot, "snapshot"),
        (TopicFrameKind::Replay, "replay"),
        (TopicFrameKind::Live, "live"),
    ] {
        let chunks = frame.event_chunks(kind);
        assert_eq!(chunks[1].as_ptr(), frame.envelope_metadata_bytes.as_ptr());
        assert_eq!(chunks[2].as_ptr(), frame.payload_bytes.as_ptr());
        let wire = chunks.concat();
        let envelope: Value = serde_json::from_slice(&wire[6..wire.len() - 2])
            .expect("SSE data contains a JSON envelope");
        assert_eq!(envelope["type"], expected_kind);
        assert_eq!(envelope["cursor"], 7);
        assert_eq!(envelope["payload"]["value"], 42);
    }
    assert_eq!(
        frame.retained_bytes(),
        frame.envelope_metadata_bytes.len() + frame.payload_bytes.len()
    );
}

#[test]
fn timeseries_delta_payload_replaces_one_point_and_preserves_wire_metadata() {
    let previous = json!({
        "rangeStart": "2026-10-12T00:00:00Z",
        "rangeEnd": "2026-10-12T00:01:00Z",
        "snapshotId": 10,
        "bucketSeconds": 60,
        "points": [{"bucketStart": "2026-10-12T00:00:00Z", "total": 1}]
    });
    let next = json!({
        "rangeStart": "2026-10-12T00:00:00Z",
        "rangeEnd": "2026-10-12T00:02:00Z",
        "snapshotId": 11,
        "bucketSeconds": 60,
        "points": [{"bucketStart": "2026-10-12T00:00:00Z", "total": 2}]
    });
    let delta = build_timeseries_delta_payload(
        &serde_json::to_vec(&previous).expect("previous payload"),
        &serde_json::to_vec(&next).expect("next payload"),
    )
    .expect("single point delta");
    assert_eq!(
        serde_json::from_slice::<Value>(&delta).expect("delta json"),
        json!({
            "kind": "timeseries-point/v1",
            "rangeEnd": "2026-10-12T00:02:00Z",
            "snapshotId": 11,
            "pointIndex": 0,
            "point": {"bucketStart": "2026-10-12T00:00:00Z", "total": 2}
        })
    );

    let topic = SubscriptionTopic::TimeseriesOpenWindow {
        range: "today".to_string(),
        time_zone: "UTC".to_string(),
        bucket: None,
        settlement_hour: None,
        upstream_account_id: None,
    };
    let frame = serialize_topic_delta_frame(
        topic.descriptor(),
        topic.cache_key().expect("topic key"),
        topic.schema_epoch(),
        8,
        7,
        delta,
    )
    .expect("delta frame");
    let chunks = frame.event_chunks(TopicFrameKind::Live);
    let wire = chunks.concat();
    let envelope: Value = serde_json::from_slice(&wire[6..wire.len() - 2]).expect("wire envelope");
    assert_eq!(envelope["type"], "live");
    assert_eq!(envelope["delivery"], "delta");
    assert_eq!(envelope["baseCursor"], 7);
    assert_eq!(envelope["cursor"], 8);
}

#[test]
fn timeseries_delta_payload_falls_back_when_multiple_points_change() {
    let previous = json!({
        "rangeEnd": "2026-10-12T00:01:00Z",
        "snapshotId": 10,
        "points": [{"total": 1}, {"total": 1}]
    });
    let next = json!({
        "rangeEnd": "2026-10-12T00:02:00Z",
        "snapshotId": 11,
        "points": [{"total": 2}, {"total": 2}]
    });
    assert!(
        build_timeseries_delta_payload(
            &serde_json::to_vec(&previous).expect("previous payload"),
            &serde_json::to_vec(&next).expect("next payload"),
        )
        .is_none()
    );
}

#[test]
fn timeseries_delta_payload_handles_metadata_only_refreshes() {
    let previous = json!({
        "rangeEnd": "2026-10-12T00:01:00Z",
        "snapshotId": 10,
        "points": [{"total": 1}]
    });
    let next = json!({
        "rangeEnd": "2026-10-12T00:02:00Z",
        "snapshotId": 11,
        "points": [{"total": 1}]
    });
    let delta = build_timeseries_delta_payload(
        &serde_json::to_vec(&previous).expect("previous payload"),
        &serde_json::to_vec(&next).expect("next payload"),
    )
    .expect("metadata delta");
    let delta: Value = serde_json::from_slice(&delta).expect("delta json");
    assert_eq!(delta["pointIndex"], Value::Null);
    assert_eq!(delta["point"], Value::Null);
}

#[test]
fn working_conversations_delta_replaces_one_conversation_and_metadata() {
    let previous = json!({
        "hasMore": false,
        "conversations": [{"promptCacheKey": "alpha", "requestCount": 1}, {"promptCacheKey": "beta", "requestCount": 1}]
    });
    let next = json!({
        "hasMore": true,
        "conversations": [{"promptCacheKey": "alpha", "requestCount": 2}, {"promptCacheKey": "beta", "requestCount": 1}]
    });
    let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size: 20,
        recent_invocation_limit: 10,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    };
    let delta = build_dashboard_delta_payload(
        &topic,
        &serde_json::to_vec(&previous).expect("previous payload"),
        &serde_json::to_vec(&next).expect("next payload"),
    )
    .expect("working conversations delta");
    assert_eq!(
        serde_json::from_slice::<Value>(&delta).expect("delta json"),
        json!({
            "kind": "collection-item/v1",
            "collection": "conversations",
            "itemIndex": 0,
            "item": {"promptCacheKey": "alpha", "requestCount": 2},
            "fields": {"hasMore": true}
        })
    );
}

#[test]
fn unchanged_refresh_clears_scheduled_flag_for_the_next_terminal_event() {
    let topic = summary_topic();
    let mut cached = seeded_cached_topic(topic, &[], Utc::now());
    cached.refresh_scheduled = true;
    cached.prompt_cache_reconcile_required = true;
    cached.prompt_cache_pressure_deferred = true;
    cached
        .prompt_cache_pending_key_hydrations
        .insert("pending-key".to_string());
    cached.prompt_cache_candidate_refill_required = true;
    cached.prompt_cache_key_hydration_scheduled = true;
    let payload = serde_json::to_vec(&cached.snapshot_payload).expect("cached payload");

    let reused = reuse_unchanged_cached_topic(&mut cached, &payload);

    assert!(reused.is_some());
    assert!(!cached.refresh_scheduled);
    assert!(!cached.prompt_cache_reconcile_required);
    assert!(!cached.prompt_cache_pressure_deferred);
    assert!(cached.prompt_cache_pending_key_hydrations.is_empty());
    assert!(!cached.prompt_cache_candidate_refill_required);
    assert!(!cached.prompt_cache_key_hydration_scheduled);
    cached.refresh_scheduled = true;
    assert!(
        cached.refresh_scheduled,
        "a later terminal event can schedule again"
    );
}

#[test]
fn dirty_unchanged_refresh_clears_dirty_without_advancing_cursor() {
    let topic = summary_topic();
    let mut cached = seeded_cached_topic(topic, &[], Utc::now());
    cached.dirty = true;
    cached.refresh_scheduled = true;
    let cursor = cached.cursor;
    let payload = serde_json::to_vec(&cached.snapshot_payload).expect("cached payload");

    let reused = reuse_unchanged_cached_topic(&mut cached, &payload);

    assert!(reused.is_some());
    assert!(!cached.dirty);
    assert!(!cached.refresh_scheduled);
    assert_eq!(cached.cursor, cursor);
}

#[test]
fn reused_prompt_cache_baseline_refreshes_reconciliation_bookkeeping() {
    let topic = summary_topic();
    let mut cached = seeded_cached_topic(topic, &[], Utc::now());
    cached.prompt_cache_full_hydration_count = 4;
    cached.prompt_cache_baseline_row_id = 12;
    cached.prompt_cache_response_source = "memory";
    let applied_terminal_ids = HashSet::from(["invoke\0occurred-at".to_string()]);

    finish_prompt_cache_baseline_reuse(
        &mut cached,
        &PromptCacheBaselineBuild {
            baseline_row_id: 37,
            persisted_identities: HashSet::new(),
            runtime_overlay_terminal_identities: HashSet::new(),
        },
        &applied_terminal_ids,
    );

    assert_eq!(cached.prompt_cache_full_hydration_count, 5);
    assert!(cached.prompt_cache_baseline_at.is_some());
    assert_eq!(cached.prompt_cache_baseline_row_id, 37);
    assert_eq!(cached.prompt_cache_response_source, "database_reconcile");
    assert_eq!(
        cached.prompt_cache_applied_terminal_ids,
        applied_terminal_ids
    );
}

#[test]
fn prune_replay_window_drops_expired_entries() {
    let now = Utc::now();
    let topic = summary_topic();
    let descriptor = topic.descriptor();
    let schema_epoch = topic.schema_epoch();
    let mut events = VecDeque::from([
        test_replay_event(
            &descriptor,
            &schema_epoch,
            1,
            now - ChronoDuration::seconds(SUBSCRIPTION_REPLAY_WINDOW_SECS + 5),
            32,
        ),
        test_replay_event(&descriptor, &schema_epoch, 2, now, 32),
    ]);
    let mut total_bytes = 64usize;

    prune_replay_window(&mut events, &mut total_bytes);

    assert_eq!(events.len(), 1);
    assert_eq!(events.front().map(|event| event.frame.cursor), Some(2));
    assert_eq!(total_bytes, 32);
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
async fn subscription_connection_isolates_unavailable_summary_from_dashboard_activity() {
    let state =
        crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap()).await;
    let hub = SubscriptionHub::new();
    let summary = summary_topic();
    let activity = SubscriptionTopic::DashboardActivityCurrent {
        range: "today".to_string(),
        time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
        recent_limit: 16,
        include_accounts: true,
        include_recent: true,
    };
    let activity_key = activity.cache_key().expect("activity topic key");
    hub.state.lock().await.topics.insert(
        activity_key,
        seeded_cached_topic(activity.clone(), &[4], Utc::now()),
    );
    state.pool.close().await;

    let prepared = hub
        .prepare_connection(
            state,
            vec![summary.descriptor(), activity.descriptor()],
            Vec::new(),
        )
        .await
        .expect("unavailable Summary must not fail the whole connection");

    assert_eq!(prepared.initial.len(), 2);
    assert_eq!(
        prepared.outcomes[0].disposition,
        TopicInitDisposition::Unavailable
    );
    assert_eq!(prepared.outcomes[0].cursor, 0);
    assert_eq!(
        prepared.outcomes[1].disposition,
        TopicInitDisposition::SnapshotNoResume
    );
    assert!(
        !prepared
            .last_sent_cursors
            .contains_key(&summary.cache_key().expect("summary topic key"))
    );
    let unavailable = prepared
        .initial
        .iter()
        .find(|frame| frame.kind == TopicFrameKind::Unavailable)
        .expect("Summary unavailable frame");
    let wire = unavailable
        .frame
        .event_chunks(TopicFrameKind::Unavailable)
        .concat();
    let envelope: Value =
        serde_json::from_slice(&wire[6..wire.len() - 2]).expect("unavailable wire envelope");
    assert_eq!(envelope["type"], "unavailable");
    assert_eq!(envelope["errorCode"], "unavailable");
    assert!(envelope.get("payload").is_none());
    assert!(envelope.get("cursor").is_none());
    assert_eq!(envelope["topic"]["topic"], "stats.summary.current");
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
    let summary = payload
        .get("summary")
        .and_then(Value::as_object)
        .expect("summary object");
    assert_eq!(
        summary.get("currentFirstResponseByteTotalAvgMs"),
        None,
        "summary stale currentFirstResponseByteTotalAvgMs should be removed",
    );
    assert_eq!(
        summary.get("currentFirstTokenAvgMs"),
        None,
        "summary stale currentFirstTokenAvgMs should be removed",
    );
    assert_eq!(
        summary.get("currentAvgTotalMs"),
        None,
        "summary stale currentAvgTotalMs should be removed",
    );
    assert_eq!(
        summary.get("currentAvgResponseMs"),
        None,
        "summary stale currentAvgResponseMs should be removed",
    );

    let account = payload
        .get("accounts")
        .and_then(Value::as_array)
        .and_then(|accounts| accounts.first())
        .and_then(Value::as_object)
        .expect("account object");
    assert_eq!(
        account.get("currentFirstResponseByteTotalAvgMs"),
        None,
        "account stale currentFirstResponseByteTotalAvgMs should be removed",
    );
    assert_eq!(
        account.get("currentFirstTokenAvgMs"),
        None,
        "account stale currentFirstTokenAvgMs should be removed",
    );
    assert_eq!(
        account.get("currentAvgTotalMs"),
        None,
        "account stale currentAvgTotalMs should be removed",
    );
    assert_eq!(
        account.get("currentAvgResponseMs"),
        None,
        "account stale currentAvgResponseMs should be removed",
    );
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
