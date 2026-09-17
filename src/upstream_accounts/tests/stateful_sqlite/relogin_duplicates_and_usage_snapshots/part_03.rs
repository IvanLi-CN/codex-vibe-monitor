use super::*;
use serde_json::json;

#[test]
pub(crate) fn kaisoumail_attach_status_is_not_readable_only_for_permission_and_missing() {
    assert!(kaisoumail_attach_status_is_not_readable(
        reqwest::StatusCode::FORBIDDEN
    ));
    assert!(kaisoumail_attach_status_is_not_readable(
        reqwest::StatusCode::NOT_FOUND
    ));
    assert!(!kaisoumail_attach_status_is_not_readable(
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    ));
    assert!(!kaisoumail_attach_status_is_not_readable(
        reqwest::StatusCode::GATEWAY_TIMEOUT
    ));
}

#[tokio::test]
pub(crate) async fn create_oauth_mailbox_session_accepts_supported_domain_variants_for_existing_mailbox()
 {
    let harness = spawn_kaisoumail_test_harness(
        "@707079.XYZ, 707979.xyz",
        vec![(
            "email_existing".to_string(),
            "finance.lab.d5r@mail-tw.707079.xyz".to_string(),
            Some("2026-06-01T00:00:00.000Z".to_string()),
        )],
    )
    .await;
    let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
        "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
    }))
    .expect("deserialize mailbox request");

    let Json(response) = create_oauth_mailbox_session(
        State(harness.state.clone()),
        HeaderMap::new(),
        Json(payload),
    )
    .await
    .expect("create mailbox session");

    assert!(response.supported);
    assert_eq!(response.email_address, "finance.lab.d5r@mail-tw.707079.xyz");
    assert_eq!(
        response.source.as_deref(),
        Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
    );
    let session_id = response.session_id.expect("session id");
    let row = load_oauth_mailbox_session(&harness.state.pool, &session_id)
        .await
        .expect("load mailbox session")
        .expect("stored mailbox session");
    assert_eq!(
        row.mailbox_source.as_deref(),
        Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
    );
    assert!(
        harness.stub.generated_requests.lock().await.is_empty(),
        "existing readable mailbox should not be recreated"
    );

    harness.abort();
}

#[tokio::test]
pub(crate) async fn create_oauth_mailbox_session_lets_kaisoumail_generate_address_upstream() {
    let harness = spawn_kaisoumail_test_harness("@707079.xyz", Vec::new()).await;
    let payload: CreateOauthMailboxSessionRequest =
        serde_json::from_value(json!({})).expect("deserialize mailbox request");

    let Json(response) = create_oauth_mailbox_session(
        State(harness.state.clone()),
        HeaderMap::new(),
        Json(payload),
    )
    .await
    .expect("create mailbox session");

    assert!(response.supported);
    assert_eq!(
        response.email_address,
        "upstream-generated-1@mailbox.kaisoumail.test"
    );
    assert_eq!(
        response.source.as_deref(),
        Some(OAUTH_MAILBOX_SOURCE_GENERATED)
    );
    let create_requests = harness.stub.create_requests.lock().await.clone();
    assert_eq!(create_requests, vec![json!({ "expiresInMinutes": 60 })]);
    let session_id = response.session_id.expect("session id");
    let row = load_oauth_mailbox_session(&harness.state.pool, &session_id)
        .await
        .expect("load mailbox session")
        .expect("stored mailbox session");
    assert_eq!(row.remote_email_id, "generated_1");
    assert_eq!(
        row.email_address,
        "upstream-generated-1@mailbox.kaisoumail.test"
    );
    assert_eq!(row.email_domain, "mailbox.kaisoumail.test");

    harness.abort();
}

#[tokio::test]
pub(crate) async fn create_oauth_mailbox_session_restores_expired_existing_mailbox_before_attach() {
    let harness = spawn_kaisoumail_test_harness(
        "@707079.xyz",
        vec![(
            "email_expired".to_string(),
            "finance.lab.d5r@mail-tw.707079.xyz".to_string(),
            Some("2026-03-20T12:50:00.000Z".to_string()),
        )],
    )
    .await;
    let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
        "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
    }))
    .expect("deserialize mailbox request");

    let Json(response) = create_oauth_mailbox_session(
        State(harness.state.clone()),
        HeaderMap::new(),
        Json(payload),
    )
    .await
    .expect("create mailbox session");

    assert!(response.supported);
    assert_eq!(
        response.source.as_deref(),
        Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
    );
    assert_eq!(response.expires_at.as_deref(), Some("2026-06-01T00:00:00Z"));
    let session_id = response.session_id.expect("session id");
    let row = load_oauth_mailbox_session(&harness.state.pool, &session_id)
        .await
        .expect("load mailbox session")
        .expect("stored mailbox session");
    assert_eq!(row.remote_email_id, "email_expired");
    assert_eq!(row.expires_at, "2026-06-01T00:00:00Z");
    assert!(
        harness.stub.generated_requests.lock().await.is_empty(),
        "restoring an existing mailbox should not create a new one"
    );

    harness.abort();
}

#[tokio::test]
pub(crate) async fn create_oauth_mailbox_session_creates_missing_supported_mailbox() {
    let harness = spawn_kaisoumail_test_harness("@707079.xyz", Vec::new()).await;
    let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
        "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
    }))
    .expect("deserialize mailbox request");

    let Json(response) = create_oauth_mailbox_session(
        State(harness.state.clone()),
        HeaderMap::new(),
        Json(payload),
    )
    .await
    .expect("create mailbox session");

    assert!(response.supported);
    assert_eq!(response.email_address, "finance.lab.d5r@mail-tw.707079.xyz");
    assert_eq!(
        response.source.as_deref(),
        Some(OAUTH_MAILBOX_SOURCE_GENERATED)
    );
    let generated_requests = harness.stub.generated_requests.lock().await.clone();
    assert_eq!(
        generated_requests,
        vec![(
            "finance.lab.d5r".to_string(),
            "mail-tw.707079.xyz".to_string()
        )]
    );
    let session_id = response.session_id.expect("session id");
    let row = load_oauth_mailbox_session(&harness.state.pool, &session_id)
        .await
        .expect("load mailbox session")
        .expect("stored mailbox session");
    assert_eq!(
        row.mailbox_source.as_deref(),
        Some(OAUTH_MAILBOX_SOURCE_GENERATED)
    );
    assert_eq!(row.email_address, "finance.lab.d5r@mail-tw.707079.xyz");

    harness.abort();
}

#[tokio::test]
pub(crate) async fn create_oauth_mailbox_session_rejects_true_unsupported_domains() {
    let harness = spawn_kaisoumail_test_harness("707979.xyz", Vec::new()).await;
    let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
        "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
    }))
    .expect("deserialize mailbox request");

    let Json(response) = create_oauth_mailbox_session(
        State(harness.state.clone()),
        HeaderMap::new(),
        Json(payload),
    )
    .await
    .expect("create mailbox session");

    assert!(!response.supported);
    assert_eq!(response.reason.as_deref(), Some("unsupported_domain"));
    assert!(
        harness.stub.generated_requests.lock().await.is_empty(),
        "unsupported domains must not trigger remote mailbox creation"
    );

    harness.abort();
}

#[tokio::test]
pub(crate) async fn delete_oauth_mailbox_session_deletes_remote_for_generated_manual_mailbox() {
    let harness = spawn_kaisoumail_test_harness("@707079.xyz", Vec::new()).await;
    let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
        "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
    }))
    .expect("deserialize mailbox request");
    let Json(created) = create_oauth_mailbox_session(
        State(harness.state.clone()),
        HeaderMap::new(),
        Json(payload),
    )
    .await
    .expect("create mailbox session");
    let session_id = created.session_id.expect("session id");
    let row = load_oauth_mailbox_session(&harness.state.pool, &session_id)
        .await
        .expect("load mailbox session")
        .expect("stored mailbox session");

    let status = delete_oauth_mailbox_session(
        State(harness.state.clone()),
        HeaderMap::new(),
        AxumPath(session_id.clone()),
    )
    .await
    .expect("delete mailbox session");

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        harness.stub.deleted_ids.lock().await.clone(),
        vec![row.remote_email_id]
    );
    assert!(
        load_oauth_mailbox_session(&harness.state.pool, &session_id)
            .await
            .expect("load mailbox session after delete")
            .is_none()
    );

    harness.abort();
}

#[tokio::test]
pub(crate) async fn cleanup_expired_oauth_mailbox_sessions_deletes_remote_for_generated_manual_mailbox()
 {
    let harness = spawn_kaisoumail_test_harness(
        "@707079.xyz",
        vec![(
            "generated_1".to_string(),
            "finance.lab.d5r@mail-tw.707079.xyz".to_string(),
            None,
        )],
    )
    .await;
    sqlx::query(
            r#"
            INSERT INTO pool_oauth_mailbox_sessions (
                session_id, remote_email_id, email_address, email_domain, mailbox_source,
                latest_code_value, latest_code_source, latest_code_updated_at, invite_subject,
                invite_copy_value, invite_copy_label, invite_updated_at, invited, last_message_id,
                created_at, updated_at, expires_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, NULL, ?6, ?6, ?7)
            "#,
        )
        .bind("expired_manual_generated")
        .bind("generated_1")
        .bind("finance.lab.d5r@mail-tw.707079.xyz")
        .bind("mail-tw.707079.xyz")
        .bind(OAUTH_MAILBOX_SOURCE_GENERATED)
        .bind("2026-03-17T00:00:00Z")
        .bind("2026-03-17T00:01:00Z")
        .execute(&harness.state.pool)
        .await
        .expect("insert expired mailbox session");

    cleanup_expired_oauth_mailbox_sessions(harness.state.as_ref())
        .await
        .expect("cleanup expired mailbox sessions");

    assert_eq!(
        harness.stub.deleted_ids.lock().await.clone(),
        vec!["generated_1".to_string()]
    );
    assert!(
        load_oauth_mailbox_session(&harness.state.pool, "expired_manual_generated")
            .await
            .expect("load cleaned mailbox session")
            .is_none()
    );

    harness.abort();
}

#[test]
pub(crate) fn collect_unseen_mailbox_messages_stops_at_last_seen_id() {
    let messages = vec![
        KaisouMailMessageSummary {
            id: "msg_3".to_string(),
            subject: Some("newest".to_string()),
            received_at: Some("2026-03-16T03:00:00Z".to_string()),
        },
        KaisouMailMessageSummary {
            id: "msg_2".to_string(),
            subject: Some("baseline".to_string()),
            received_at: Some("2026-03-16T02:00:00Z".to_string()),
        },
        KaisouMailMessageSummary {
            id: "msg_1".to_string(),
            subject: Some("older".to_string()),
            received_at: Some("2026-03-16T01:00:00Z".to_string()),
        },
    ];

    let unseen = collect_unseen_mailbox_messages(messages, Some("msg_2"));

    assert_eq!(unseen.len(), 1);
    assert_eq!(unseen[0].id, "msg_3");
}

#[test]
pub(crate) fn collect_unseen_mailbox_messages_keeps_all_when_baseline_is_missing() {
    let messages = vec![
        KaisouMailMessageSummary {
            id: "msg_2".to_string(),
            subject: None,
            received_at: Some("2026-03-16T02:00:00Z".to_string()),
        },
        KaisouMailMessageSummary {
            id: "msg_1".to_string(),
            subject: None,
            received_at: Some("2026-03-16T01:00:00Z".to_string()),
        },
    ];

    let unseen = collect_unseen_mailbox_messages(messages.clone(), Some("missing"));

    assert_eq!(unseen.len(), messages.len());
    assert_eq!(unseen[0].id, "msg_2");
    assert_eq!(unseen[1].id, "msg_1");
}

#[test]
pub(crate) fn next_mailbox_cursor_after_refresh_advances_to_latest_processed_message() {
    let processed = vec![
        KaisouMailMessageSummary {
            id: "msg_5".to_string(),
            subject: Some("latest".to_string()),
            received_at: Some("2026-03-16T05:00:00Z".to_string()),
        },
        KaisouMailMessageSummary {
            id: "msg_4".to_string(),
            subject: Some("older".to_string()),
            received_at: Some("2026-03-16T04:00:00Z".to_string()),
        },
    ];

    let next = next_mailbox_cursor_after_refresh(Some("msg_3"), &processed);

    assert_eq!(next.as_deref(), Some("msg_5"));
}

#[test]
pub(crate) fn next_mailbox_cursor_after_refresh_keeps_existing_cursor_when_nothing_was_processed() {
    let next = next_mailbox_cursor_after_refresh(Some("msg_3"), &[]);

    assert_eq!(next.as_deref(), Some("msg_3"));
}

#[test]
pub(crate) fn merge_mailbox_code_prefers_fresher_refresh_value() {
    let stored = ParsedMailboxCode {
        value: "111111".to_string(),
        source: "subject".to_string(),
        updated_at: "2026-03-16T00:00:00Z".to_string(),
    };
    let fresh = ParsedMailboxCode {
        value: "222222".to_string(),
        source: "subject".to_string(),
        updated_at: "2026-03-16T00:01:00Z".to_string(),
    };

    let merged = merge_mailbox_code(Some(fresh), Some(stored)).expect("merged code");

    assert_eq!(merged.value, "222222");
    assert_eq!(merged.updated_at, "2026-03-16T00:01:00Z");
}

#[test]
pub(crate) fn merge_mailbox_invite_keeps_newer_stored_value_when_refresh_is_older() {
    let stored = ParsedMailboxInvite {
        subject: "New invite".to_string(),
        copy_value: "https://example.com/new".to_string(),
        copy_label: "invite-link".to_string(),
        updated_at: "2026-03-16T00:05:00Z".to_string(),
    };
    let fresh = ParsedMailboxInvite {
        subject: "Old invite".to_string(),
        copy_value: "https://example.com/old".to_string(),
        copy_label: "invite-link".to_string(),
        updated_at: "2026-03-16T00:01:00Z".to_string(),
    };

    let merged = merge_mailbox_invite(Some(fresh), Some(stored)).expect("merged invite");

    assert_eq!(merged.subject, "New invite");
    assert_eq!(merged.copy_value, "https://example.com/new");
}

#[test]
pub(crate) fn random_base36_uses_letters_and_digits() {
    let token = random_base36(24).expect("base36 token");
    assert_eq!(token.len(), 24);
    assert!(
        token
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
    );
    assert!(token.chars().any(|ch| ch.is_ascii_lowercase()));
    assert!(token.chars().any(|ch| ch.is_ascii_digit()));
}

#[test]
pub(crate) fn build_window_usage_range_aligns_to_current_reset_window() {
    let now = parse_rfc3339_utc("2026-03-30T12:30:00Z").expect("fixed now");
    let range =
        build_window_usage_range(now, 300, Some("2026-03-30T14:00:00Z")).expect("aligned range");

    assert_eq!(
        range.start_at,
        parse_rfc3339_utc("2026-03-30T09:00:00Z").expect("expected start")
    );
    assert_eq!(range.end_at, now);
}

#[test]
pub(crate) fn build_window_usage_range_reuses_stale_reset_window_bounds() {
    let now = parse_rfc3339_utc("2026-03-30T12:30:00Z").expect("fixed now");
    let range =
        build_window_usage_range(now, 300, Some("2026-03-29T23:00:00Z")).expect("historical range");

    assert_eq!(
        range.start_at,
        parse_rfc3339_utc("2026-03-29T18:00:00Z").expect("expected historical start")
    );
    assert_eq!(
        range.end_at,
        parse_rfc3339_utc("2026-03-29T23:00:00Z").expect("expected historical end")
    );
}

#[tokio::test]
pub(crate) async fn enrich_window_actual_usage_for_summaries_counts_live_window_rows() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_oauth_account(&state.pool, "Live Usage OAuth").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(27.0),
        Some(61.0),
    )
    .await;

    let primary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(45));
    let secondary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
    let failed_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(10));

    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &primary_row_at,
        Some(2400),
        Some(1200),
        Some(600),
        Some(4200),
        Some(0.042),
    )
    .await;
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &secondary_row_at,
        Some(1000),
        Some(500),
        Some(250),
        Some(1750),
        Some(0.0175),
    )
    .await;
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &failed_row_at,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id + 999,
        &primary_row_at,
        Some(999),
        Some(999),
        Some(999),
        Some(2997),
        Some(0.2997),
    )
    .await;

    let mut summaries = load_upstream_account_summaries(&state.pool, &state.config)
        .await
        .expect("load upstream account summaries");
    enrich_window_actual_usage_for_summaries(state.as_ref(), &mut summaries)
        .await
        .expect("enrich actual usage");

    let summary = summaries
        .into_iter()
        .find(|item| item.id == account_id)
        .expect("summary exists");
    let primary_usage = summary
        .primary_window
        .and_then(|window| window.actual_usage)
        .expect("primary actual usage");
    let secondary_usage = summary
        .secondary_window
        .and_then(|window| window.actual_usage)
        .expect("secondary actual usage");

    assert_eq!(primary_usage.request_count, 2);
    assert_eq!(primary_usage.total_tokens, 4200);
    assert_eq!(primary_usage.input_tokens, 2400);
    assert_eq!(primary_usage.output_tokens, 1200);
    assert_eq!(primary_usage.cache_input_tokens, 600);
    assert_cost_close(primary_usage.total_cost, 0.042);

    assert_eq!(secondary_usage.request_count, 3);
    assert_eq!(secondary_usage.total_tokens, 5950);
    assert_eq!(secondary_usage.input_tokens, 3400);
    assert_eq!(secondary_usage.output_tokens, 1700);
    assert_eq!(secondary_usage.cache_input_tokens, 850);
    assert_cost_close(secondary_usage.total_cost, 0.0595);
}

#[tokio::test]
pub(crate) async fn enrich_window_actual_usage_for_summaries_uses_matching_stale_reset_window() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = 402_i64;
    let reset_at = Utc::now() - ChronoDuration::hours(10);
    let mut summary = test_summary_with_statuses(
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
        UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
    );
    summary.id = account_id;
    summary.primary_window = Some(RateWindowSnapshot {
        used_percent: 100.0,
        used_text: "100% used".to_string(),
        limit_text: "5h window".to_string(),
        resets_at: Some(format_utc_iso(reset_at)),
        window_duration_mins: 300,
        actual_usage: None,
    });

    let inside_window_at = shanghai_local_iso(reset_at - ChronoDuration::hours(1));
    let before_window_at = shanghai_local_iso(reset_at - ChronoDuration::hours(6));
    let after_window_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(30));

    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &inside_window_at,
        Some(1800),
        Some(900),
        Some(450),
        Some(3150),
        Some(0.0315),
    )
    .await;
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &before_window_at,
        Some(3000),
        Some(1200),
        Some(600),
        Some(4800),
        Some(0.048),
    )
    .await;
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &after_window_at,
        Some(500),
        Some(250),
        Some(100),
        Some(850),
        Some(0.0085),
    )
    .await;

    let mut items = vec![summary];
    enrich_window_actual_usage_for_summaries(state.as_ref(), &mut items)
        .await
        .expect("enrich stale window actual usage");

    let usage = items[0]
        .primary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("stale primary actual usage");
    assert_eq!(usage.request_count, 1);
    assert_eq!(usage.total_tokens, 3150);
    assert_eq!(usage.input_tokens, 1800);
    assert_eq!(usage.output_tokens, 900);
    assert_eq!(usage.cache_input_tokens, 450);
    assert_cost_close(usage.total_cost, 0.0315);
}

#[tokio::test]
pub(crate) async fn enrich_window_actual_usage_for_summaries_reads_materialized_archive_usage_past_retention_cutoff()
 {
    let mut config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
    config.invocation_max_days = 1;
    config.archive_dir = crate::tests::test_runtime_path(&format!(
        "archive-tests/window-actual-usage-{}",
        random_base36(8).expect("archive suffix")
    ));
    let state = test_app_state_with_config_and_parallelism(
        config,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = 401_i64;
    let mut summary = test_summary_with_statuses(
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE,
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
        UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
    );
    summary.id = account_id;
    summary.primary_window = Some(RateWindowSnapshot {
        used_percent: 12.0,
        used_text: "12% used".to_string(),
        limit_text: "3d rolling window".to_string(),
        resets_at: None,
        window_duration_mins: 60 * 24 * 3,
        actual_usage: None,
    });

    let live_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::hours(6));
    let archived_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &live_row_at,
        Some(1800),
        Some(900),
        Some(300),
        Some(3000),
        Some(0.03),
    )
    .await;
    let archived_bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_row_at).expect("archived usage bucket epoch");
    sqlx::query(
        r#"
            INSERT INTO upstream_account_usage_hourly (
                bucket_start_epoch,
                upstream_account_id,
                request_count,
                total_tokens,
                total_cost,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                first_seen_at,
                last_seen_at,
                updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now')
            )
            "#,
    )
    .bind(archived_bucket_start_epoch)
    .bind(account_id)
    .bind(1_i64)
    .bind(2000_i64)
    .bind(0.02_f64)
    .bind(1200_i64)
    .bind(600_i64)
    .bind(200_i64)
    .bind(&archived_row_at)
    .bind(&archived_row_at)
    .execute(&state.pool)
    .await
    .expect("insert materialized archived usage hourly row");

    let mut items = vec![summary];
    enrich_window_actual_usage_for_summaries(state.as_ref(), &mut items)
        .await
        .expect("enrich actual usage with materialized archive usage");

    let usage = items[0]
        .primary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("primary actual usage");
    assert_eq!(usage.request_count, 2);
    assert_eq!(usage.total_tokens, 5000);
    assert_eq!(usage.input_tokens, 3000);
    assert_eq!(usage.output_tokens, 1500);
    assert_eq!(usage.cache_input_tokens, 500);
    assert_cost_close(usage.total_cost, 0.05);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_populates_upstream_account_usage_hourly_from_archive()
 {
    let mut config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
    config.invocation_max_days = 1;
    config.archive_dir = crate::tests::test_runtime_path(&format!(
        "archive-tests/upstream-account-usage-hourly-{}",
        random_base36(8).expect("archive suffix")
    ));
    let state = test_app_state_with_config_and_parallelism(
        config,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = 587_i64;
    let archived_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
    seed_window_actual_usage_archive_batch(
        &state.pool,
        &state.config.archive_dir,
        "materialize-upstream-account-usage-hourly",
        &[(
            account_id,
            archived_row_at.clone(),
            Some(1200),
            Some(600),
            Some(200),
            Some(2000),
            Some(0.02),
        )],
    )
    .await;

    let summary = materialize_historical_rollups(&state.pool, &state.config, false)
        .await
        .expect("materialize historical rollups");
    assert_eq!(summary.materialized_invocation_batches, 1);

    let bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_row_at).expect("archive bucket start");
    let row = sqlx::query_as::<_, (i64, i64, i64, i64, f64, i64, i64, i64)>(
        r#"
            SELECT
                bucket_start_epoch,
                upstream_account_id,
                request_count,
                total_tokens,
                total_cost,
                input_tokens,
                output_tokens,
                cache_input_tokens
            FROM upstream_account_usage_hourly
            WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2
            "#,
    )
    .bind(bucket_start_epoch)
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load upstream account usage hourly row");
    assert_eq!(row.2, 1);
    assert_eq!(row.3, 2000);
    assert_eq!(row.5, 1200);
    assert_eq!(row.6, 600);
    assert_eq!(row.7, 200);
    assert_cost_close(row.4, 0.02);
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_keeps_actual_usage_null_until_batch_hydrate() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Roster Usage").await;
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 60 * 24,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        secondary: None,
        credits: None,
    };
    persist_usage_snapshot(&state.pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist roster usage snapshot");
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::hours(1)),
        Some(2100),
        Some(900),
        Some(300),
        Some(3300),
        Some(0.033),
    )
    .await;

    let Json(response) =
        list_upstream_accounts(State(state), Query(ListUpstreamAccountsQuery::default()))
            .await
            .expect("list upstream accounts");
    let account = response
        .items
        .into_iter()
        .find(|item| item.id == account_id)
        .expect("account in roster response");
    assert!(
        account
            .primary_window
            .as_ref()
            .and_then(|window| window.actual_usage)
            .is_none(),
        "roster response should leave actual usage null until batch hydrate",
    );
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_returns_batch_actual_usage() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage").await;
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 60 * 24,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 9.0,
            window_duration_mins: 60 * 24 * 7,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        credits: None,
    };
    persist_usage_snapshot(&state.pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist hydrate usage snapshot");
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::hours(1)),
        Some(2100),
        Some(900),
        Some(300),
        Some(3300),
        Some(0.033),
    )
    .await;
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::days(2)),
        Some(700),
        Some(200),
        Some(100),
        Some(1000),
        Some(0.01),
    )
    .await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");
    assert_eq!(item["accountId"], account_id);
    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 3300);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 2);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 4300);
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_does_not_double_count_partial_live_rows_without_cursor()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage Double Count").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(18.0),
        Some(9.0),
    )
    .await;

    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::minutes(20)),
        Some(1200),
        Some(600),
        Some(200),
        Some(2000),
        Some(0.02),
    )
    .await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage without cursor");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 2000);
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_falls_back_to_live_raw_rows_for_missing_hourly_buckets()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id =
        insert_api_key_account(&state.pool, "Hydrate Usage Missing Hourly Bucket").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(18.0),
        Some(9.0),
    )
    .await;

    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &shanghai_local_iso(Utc::now() - ChronoDuration::hours(2)),
        Some(1200),
        Some(600),
        Some(200),
        Some(2000),
        Some(0.02),
    )
    .await;
    let cursor_id =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(&state.pool)
            .await
            .expect("load invocation cursor");
    sqlx::query(
        r#"
            INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
            VALUES (?1, ?2, datetime('now'))
            ON CONFLICT(dataset) DO UPDATE SET
                cursor_id = excluded.cursor_id,
                updated_at = datetime('now')
            "#,
    )
    .bind("codex_invocations")
    .bind(cursor_id)
    .execute(&state.pool)
    .await
    .expect("mark live rollup cursor without backfill");

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch usage with missing hourly buckets");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 2000);
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_merges_hourly_rows_when_live_cursor_missing()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage Missing Cursor").await;
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 300,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 9.0,
            window_duration_mins: 60 * 24 * 7,
            resets_at: Some((Utc::now() + ChronoDuration::hours(6)).to_rfc3339()),
        }),
        credits: None,
    };
    persist_usage_snapshot(&state.pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist hydrate usage snapshot");

    let archived_hourly_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
    insert_upstream_account_usage_hourly_row(
        &state.pool,
        UpstreamAccountUsageHourlyRow {
            account_id,
            occurred_at: &archived_hourly_at,
            request_count: 2,
            input_tokens: 2800,
            output_tokens: 1200,
            cache_input_tokens: 400,
            total_cost: 0.044,
        },
    )
    .await;

    insert_window_actual_usage_invocation(
        &state.pool,
        WindowActualUsageInvocation {
            account_id,
            occurred_at: &shanghai_local_iso(Utc::now() - ChronoDuration::hours(1)),
            input_tokens: Some(2100),
            output_tokens: Some(900),
            cache_input_tokens: Some(300),
            total_tokens: Some(3300),
            cost: Some(0.033),
        },
    )
    .await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage without live cursor");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 3300);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 3);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 7700);
    assert_eq!(item["secondaryActualUsage"]["inputTokens"], 4900);
    assert_eq!(item["secondaryActualUsage"]["outputTokens"], 2100);
    assert_eq!(item["secondaryActualUsage"]["cacheInputTokens"], 700);
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_keeps_pre_cursor_partial_minute_exact() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_oauth_account(&state.pool, "Hydrate Usage Partial Minute").await;
    let now = (Utc::now() - ChronoDuration::minutes(1))
        .with_second(30)
        .and_then(|value| value.with_nanosecond(0))
        .expect("align fixed now");
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 300,
            resets_at: Some(now.to_rfc3339()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 9.0,
            window_duration_mins: 60 * 24 * 7,
            resets_at: Some(now.to_rfc3339()),
        }),
        credits: None,
    };
    persist_usage_snapshot(&state.pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist hydrate usage snapshot");

    let boundary_row_at =
        shanghai_local_iso(now - ChronoDuration::hours(5) + ChronoDuration::seconds(15));
    let full_minute_row_at = shanghai_local_iso(now - ChronoDuration::hours(4));
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &boundary_row_at,
        Some(1000),
        Some(500),
        Some(100),
        Some(1600),
        Some(0.016),
    )
    .await;
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &full_minute_row_at,
        Some(1500),
        Some(700),
        Some(200),
        Some(2400),
        Some(0.024),
    )
    .await;

    insert_full_minute_rollup(&state.pool, account_id, &full_minute_row_at).await;

    let cursor_id =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(&state.pool)
            .await
            .expect("load invocation cursor");
    sqlx::query(
        r#"
            INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
            VALUES (?1, ?2, datetime('now'))
            ON CONFLICT(dataset) DO UPDATE SET
                cursor_id = excluded.cursor_id,
                updated_at = datetime('now')
            "#,
    )
    .bind("codex_invocations")
    .bind(cursor_id)
    .execute(&state.pool)
    .await
    .expect("mark live rollup cursor");

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage with partial minute boundary");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 2);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 4000);
    assert_eq!(item["primaryActualUsage"]["inputTokens"], 2500);
    assert_eq!(item["primaryActualUsage"]["outputTokens"], 1200);
    assert_eq!(item["primaryActualUsage"]["cacheInputTokens"], 300);
}

async fn insert_full_minute_rollup(pool: &SqlitePool, account_id: i64, occurred_at: &str) {
    sqlx::query(
        "INSERT INTO upstream_account_stats_minute (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, in_flight_count, total_tokens, input_tokens, output_tokens, cache_input_tokens, total_cost, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
    )
    .bind(invocation_bucket_start_epoch_for_seconds(occurred_at, 60).expect("minute bucket"))
    .bind(SOURCE_PROXY)
    .bind(account_id)
    .bind(1_i64)
    .bind(1_i64)
    .bind(2400_i64)
    .bind(1500_i64)
    .bind(700_i64)
    .bind(200_i64)
    .bind(0.024_f64)
    .execute(pool)
    .await
    .expect("insert minute rollup row");
}

#[tokio::test]
pub(crate) async fn get_upstream_account_window_usage_includes_archived_partial_bucket_before_retention_cutoff()
 {
    let mut config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
    config.invocation_max_days = 1;
    config.archive_dir = crate::tests::test_runtime_path(&format!(
        "archive-tests/upstream-account-usage-boundary-{}",
        random_base36(8).expect("archive suffix")
    ));
    let state = test_app_state_with_config_and_parallelism(
        config,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_api_key_account(&state.pool, "Hydrate Usage Archived Boundary").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(18.0),
        Some(9.0),
    )
    .await;

    let now = Utc::now();
    let archived_boundary_at =
        shanghai_local_iso(now - ChronoDuration::days(7) + ChronoDuration::minutes(5));
    let archived_full_hour_at = shanghai_local_iso(now - ChronoDuration::days(2));
    seed_window_actual_usage_archive_batch(
        &state.pool,
        &state.config.archive_dir,
        "window-usage-archived-boundary",
        &[
            (
                account_id,
                archived_boundary_at.clone(),
                Some(900),
                Some(500),
                Some(100),
                Some(1500),
                Some(0.015),
            ),
            (
                account_id,
                archived_full_hour_at,
                Some(2800),
                Some(1200),
                Some(400),
                Some(4400),
                Some(0.044),
            ),
        ],
    )
    .await;
    materialize_historical_rollups(&state.pool, &state.config, false)
        .await
        .expect("materialize historical rollups");

    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &shanghai_local_iso(now - ChronoDuration::minutes(20)),
        Some(1200),
        Some(600),
        Some(200),
        Some(2000),
        Some(0.02),
    )
    .await;

    let Json(response) = get_upstream_account_window_usage(
        State(state),
        Json(UpstreamAccountWindowUsageRequest {
            account_ids: vec![account_id],
        }),
    )
    .await
    .expect("load batch actual usage across retention cutoff");
    let payload = serde_json::to_value(&response).expect("serialize batch usage response");
    let item = payload["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("batch usage item");

    assert_eq!(item["primaryActualUsage"]["requestCount"], 1);
    assert_eq!(item["primaryActualUsage"]["totalTokens"], 2000);
    assert_eq!(item["secondaryActualUsage"]["requestCount"], 3);
    assert_eq!(item["secondaryActualUsage"]["totalTokens"], 7900);
    assert_eq!(item["secondaryActualUsage"]["inputTokens"], 4900);
    assert_eq!(item["secondaryActualUsage"]["outputTokens"], 2300);
    assert_eq!(item["secondaryActualUsage"]["cacheInputTokens"], 700);
}

#[tokio::test]
pub(crate) async fn load_upstream_account_detail_with_actual_usage_serializes_actual_usage_camel_case()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;

    let account_id = insert_oauth_account(&state.pool, "Detail Usage OAuth").await;
    insert_limit_sample_with_usage(
        &state.pool,
        account_id,
        &format_utc_iso(Utc::now()),
        Some(33.0),
        Some(55.0),
    )
    .await;

    let primary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(25));
    let secondary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(1));
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &primary_row_at,
        Some(2100),
        Some(900),
        Some(300),
        Some(3300),
        Some(0.033),
    )
    .await;
    insert_window_actual_usage_invocation!(
        &state.pool,
        account_id,
        &secondary_row_at,
        Some(700),
        Some(200),
        Some(100),
        Some(1000),
        Some(0.01),
    )
    .await;

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .expect("load detail with actual usage")
        .expect("detail exists");
    let primary_usage = detail
        .summary
        .primary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("primary actual usage");
    let secondary_usage = detail
        .summary
        .secondary_window
        .as_ref()
        .and_then(|window| window.actual_usage)
        .expect("secondary actual usage");

    assert_eq!(primary_usage.request_count, 1);
    assert_eq!(primary_usage.total_tokens, 3300);
    assert_cost_close(primary_usage.total_cost, 0.033);

    assert_eq!(secondary_usage.request_count, 2);
    assert_eq!(secondary_usage.total_tokens, 4300);
    assert_cost_close(secondary_usage.total_cost, 0.043);

    let payload = serde_json::to_value(&detail).expect("serialize detail payload");
    assert_eq!(payload["primaryWindow"]["actualUsage"]["requestCount"], 1);
    assert_eq!(payload["primaryWindow"]["actualUsage"]["totalTokens"], 3300);
    assert_eq!(payload["primaryWindow"]["actualUsage"]["inputTokens"], 2100);
    assert_eq!(payload["primaryWindow"]["actualUsage"]["outputTokens"], 900);
    assert_eq!(
        payload["primaryWindow"]["actualUsage"]["cacheInputTokens"],
        300
    );
    assert_eq!(payload["secondaryWindow"]["actualUsage"]["requestCount"], 2);
    assert_eq!(
        payload["secondaryWindow"]["actualUsage"]["totalTokens"],
        4300
    );
}

pub(crate) struct UpstreamAccountUsageHourlyRow<'a> {
    pub(crate) account_id: i64,
    pub(crate) occurred_at: &'a str,
    pub(crate) request_count: i64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
}

pub(crate) async fn insert_upstream_account_usage_hourly_row(
    pool: &SqlitePool,
    row: UpstreamAccountUsageHourlyRow<'_>,
) {
    let bucket_start_epoch =
        invocation_bucket_start_epoch(row.occurred_at).expect("derive usage hourly bucket");
    let total_tokens = row.input_tokens + row.output_tokens + row.cache_input_tokens;
    sqlx::query(
        r#"
            INSERT INTO upstream_account_usage_hourly (
                bucket_start_epoch,
                upstream_account_id,
                request_count,
                total_tokens,
                total_cost,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                first_seen_at,
                last_seen_at,
                updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?9
            )
            "#,
    )
    .bind(bucket_start_epoch)
    .bind(row.account_id)
    .bind(row.request_count)
    .bind(total_tokens)
    .bind(row.total_cost)
    .bind(row.input_tokens)
    .bind(row.output_tokens)
    .bind(row.cache_input_tokens)
    .bind(row.occurred_at)
    .execute(pool)
    .await
    .expect("insert upstream account usage hourly row");
}

pub(crate) fn benchmark_percentile(samples_ms: &[f64], percentile: f64) -> f64 {
    let mut sorted = samples_ms.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).expect("finite samples"));
    let rank = ((sorted.len().saturating_sub(1) as f64) * percentile).ceil() as usize;
    sorted[rank]
}

pub(crate) fn benchmark_average(samples_ms: &[f64]) -> f64 {
    samples_ms.iter().sum::<f64>() / samples_ms.len() as f64
}

pub(crate) fn round_millis(value_ms: f64) -> f64 {
    (value_ms * 100.0).round() / 100.0
}
