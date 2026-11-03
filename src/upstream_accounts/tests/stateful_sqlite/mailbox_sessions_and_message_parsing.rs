use super::*;
use serde_json::json;

#[test]
fn parse_mailbox_code_prefers_subject_match() {
    let detail = KaisouMailMessageDetail {
        id: "msg_1".to_string(),
        subject: Some("Your ChatGPT code is 612345".to_string()),
        content: Some("Ignore body 000000".to_string()),
        html: None,
        received_at: Some("2026-03-16T00:00:00Z".to_string()),
    };

    let parsed = parse_mailbox_code(&detail).expect("subject code");
    assert_eq!(parsed.value, "612345");
    assert_eq!(parsed.source, "subject");
}

#[test]
fn decode_mailbox_detail_accepts_text_and_preview_text_together() {
    let payload: KaisouMailMessageDetailPayload = serde_json::from_value(json!({
        "message": {
            "id": "msg_preview_and_text",
            "subject": "Your temporary ChatGPT verification code",
            "previewText": "Preview fallback 123456",
            "text": null,
            "html": "<p>OpenAI verification code: 654321</p>",
            "receivedAt": "2026-05-06T20:09:49.322Z"
        }
    }))
    .expect("decode message detail with both previewText and text");

    assert_eq!(payload.message.id, "msg_preview_and_text");
    assert_eq!(
        payload.message.content.as_deref(),
        Some("Preview fallback 123456")
    );
    assert_eq!(
        parse_mailbox_code(&payload.message)
            .expect("code from subject/html")
            .value,
        "654321"
    );
}

#[test]
fn kaisoumail_config_debug_redacts_api_key() {
    let config = UpstreamAccountsKaisouMailConfig {
        base_url: Url::parse("https://km.example.test").expect("url"),
        api_key: "cfm_secret_value".to_string(),
    };

    let debug = format!("{config:?}");

    assert!(debug.contains("api_key"));
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("cfm_secret_value"));
}

#[test]
fn parse_mailbox_code_falls_back_to_body_match() {
    let detail = KaisouMailMessageDetail {
        id: "msg_2".to_string(),
        subject: Some("Security notice".to_string()),
        content: Some("Use this verification code: 481122 to continue.".to_string()),
        html: None,
        received_at: Some("2026-03-16T00:00:00Z".to_string()),
    };

    let parsed = parse_mailbox_code(&detail).expect("body code");
    assert_eq!(parsed.value, "481122");
    assert_eq!(parsed.source, "content");
}

#[test]
fn parse_mailbox_code_supports_localized_subjects() {
    let detail = KaisouMailMessageDetail {
        id: "msg_zh_subject".to_string(),
        subject: Some("你的 OpenAI 代码为 438211".to_string()),
        content: Some("如果这不是你本人操作，请重置密码。".to_string()),
        html: None,
        received_at: Some("2026-03-23T23:48:33Z".to_string()),
    };

    let parsed = parse_mailbox_code(&detail).expect("localized subject code");
    assert_eq!(parsed.value, "438211");
    assert_eq!(parsed.source, "subject");
}

#[test]
fn parse_mailbox_code_supports_localized_html_and_fullwidth_digits() {
    let detail = KaisouMailMessageDetail {
        id: "msg_zh_html".to_string(),
        subject: Some("安全提醒".to_string()),
        content: None,
        html: Some(
            "<div>OpenAI</div><p>输入此临时验证码以继续：</p><strong>４３８２１１</strong>"
                .to_string(),
        ),
        received_at: Some("2026-03-24T00:00:00Z".to_string()),
    };

    let parsed = parse_mailbox_code(&detail).expect("localized html code");
    assert_eq!(parsed.value, "438211");
    assert_eq!(parsed.source, "html");
}

#[test]
fn parse_mailbox_code_prefers_digits_after_marker() {
    let detail = KaisouMailMessageDetail {
        id: "msg_order_and_code".to_string(),
        subject: Some("OpenAI order update".to_string()),
        content: Some("Order 1234. Your verification code is 567890.".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:05:30Z".to_string()),
    };

    let parsed = parse_mailbox_code(&detail).expect("verification code");
    assert_eq!(parsed.value, "567890");
    assert_eq!(parsed.source, "content");
}

#[test]
fn parse_mailbox_code_rejects_weak_subject_match_without_local_brand() {
    let detail = KaisouMailMessageDetail {
        id: "msg_weak_subject_without_local_brand".to_string(),
        subject: Some("Your code is 123456".to_string()),
        content: Some("OpenAI account activity summary".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:05:45Z".to_string()),
    };

    assert!(parse_mailbox_code(&detail).is_none());
}

#[test]
fn parse_mailbox_code_rejects_strong_subject_match_without_brand() {
    let detail = KaisouMailMessageDetail {
        id: "msg_strong_subject_without_brand".to_string(),
        subject: Some("验证码 123456".to_string()),
        content: Some("请在十分钟内完成验证。".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:05:50Z".to_string()),
    };

    assert!(parse_mailbox_code(&detail).is_none());
}

#[test]
fn parse_mailbox_code_rejects_unrelated_numbers_without_code_semantics() {
    let detail = KaisouMailMessageDetail {
        id: "msg_negative_code".to_string(),
        subject: Some("OpenAI receipt 438211".to_string()),
        content: Some("Invoice total: 23.00 USD".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:05:00Z".to_string()),
    };

    assert!(parse_mailbox_code(&detail).is_none());
}

#[test]
fn parse_mailbox_invite_extracts_workspace_link() {
    let detail = KaisouMailMessageDetail {
        id: "msg_3".to_string(),
        subject: Some("Alex has invited you to a workspace".to_string()),
        content: Some("Join workspace: https://chatgpt.com/workspace/invite/abc123".to_string()),
        html: None,
        received_at: Some("2026-03-16T00:00:00Z".to_string()),
    };

    let parsed = parse_mailbox_invite(&detail).expect("invite summary");
    assert_eq!(parsed.subject, "Alex has invited you to a workspace");
    assert_eq!(
        parsed.copy_value,
        "https://chatgpt.com/workspace/invite/abc123"
    );
    assert_eq!(parsed.copy_label, "invite-link");
}

#[test]
fn parse_mailbox_invite_supports_localized_templates() {
    let detail = KaisouMailMessageDetail {
        id: "msg_zh_invite".to_string(),
        subject: Some("Alice 邀请你加入 OpenAI 工作区".to_string()),
        content: Some("请接受邀请：https://chatgpt.com/workspace/invite/abc123".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:06:00Z".to_string()),
    };

    let parsed = parse_mailbox_invite(&detail).expect("localized invite");
    assert_eq!(parsed.subject, "Alice 邀请你加入 OpenAI 工作区");
    assert_eq!(
        parsed.copy_value,
        "https://chatgpt.com/workspace/invite/abc123"
    );
}

#[test]
fn parse_mailbox_invite_accepts_body_only_workspace_invites() {
    let detail = KaisouMailMessageDetail {
        id: "msg_body_only_invite".to_string(),
        subject: Some("OpenAI workspace update".to_string()),
        content: Some(
            "请接受邀请并加入工作区：https://chatgpt.com/workspace/invite/accept?workspace=ws_789"
                .to_string(),
        ),
        html: None,
        received_at: Some("2026-03-24T00:06:30Z".to_string()),
    };

    let parsed = parse_mailbox_invite(&detail).expect("body invite");
    assert_eq!(
        parsed.copy_value,
        "https://chatgpt.com/workspace/invite/accept?workspace=ws_789"
    );
}

#[test]
fn parse_mailbox_invite_accepts_query_driven_cta_links() {
    let detail = KaisouMailMessageDetail {
        id: "msg_query_invite".to_string(),
        subject: Some("Alice has invited you to a workspace".to_string()),
        content: Some("Open your invite: https://chatgpt.com/workspace?invite=abc123".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:06:45Z".to_string()),
    };

    let parsed = parse_mailbox_invite(&detail).expect("query invite");
    assert_eq!(
        parsed.copy_value,
        "https://chatgpt.com/workspace?invite=abc123"
    );
}

#[test]
fn parse_mailbox_invite_accepts_body_only_invites_without_workspace_keyword() {
    let detail = KaisouMailMessageDetail {
        id: "msg_body_only_plain_invite".to_string(),
        subject: Some("OpenAI account notice".to_string()),
        content: Some("Accept invitation: https://chatgpt.com/invite/abc123".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:06:50Z".to_string()),
    };

    let parsed = parse_mailbox_invite(&detail).expect("body invite without workspace");
    assert_eq!(parsed.copy_value, "https://chatgpt.com/invite/abc123");
}

#[test]
fn parse_mailbox_invite_accepts_redirect_wrapped_brand_invites() {
    let detail = KaisouMailMessageDetail {
            id: "msg_redirect_wrapped_invite".to_string(),
            subject: Some("Alex has invited you to a workspace".to_string()),
            content: Some(
                "Accept invitation: https://click.example.com/track?target=https%3A%2F%2Fchatgpt.com%2Fworkspace%2Finvite%2Fabc123".to_string(),
            ),
            html: None,
            received_at: Some("2026-03-24T00:07:10Z".to_string()),
        };

    let parsed = parse_mailbox_invite(&detail).expect("redirect wrapped invite");
    assert_eq!(
        parsed.copy_value,
        "https://chatgpt.com/workspace/invite/abc123"
    );
}

#[test]
fn parse_mailbox_invite_rejects_non_invite_workspace_links() {
    let detail = KaisouMailMessageDetail {
        id: "msg_negative_invite".to_string(),
        subject: Some("OpenAI workspace digest".to_string()),
        content: Some("Workspace docs: https://chatgpt.com/workspace".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:07:00Z".to_string()),
    };

    assert!(parse_mailbox_invite(&detail).is_none());
}

#[test]
fn parse_mailbox_invite_rejects_help_articles_about_accepting_invites() {
    let detail = KaisouMailMessageDetail {
            id: "msg_help_article".to_string(),
            subject: Some("OpenAI workspace help".to_string()),
            content: Some(
                "Need help to accept invitation to your workspace? Read https://help.openai.com/en/articles/12345-accept-invitation-to-workspace"
                    .to_string(),
            ),
            html: None,
            received_at: Some("2026-03-24T00:07:30Z".to_string()),
        };

    assert!(parse_mailbox_invite(&detail).is_none());
}

#[test]
fn parse_mailbox_invite_rejects_generic_workspace_url_even_with_invite_subject() {
    let detail = KaisouMailMessageDetail {
        id: "msg_negative_workspace_home".to_string(),
        subject: Some("Alice has invited you to a workspace".to_string()),
        content: Some("Open workspace: https://chatgpt.com/workspace".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:08:00Z".to_string()),
    };

    assert!(parse_mailbox_invite(&detail).is_none());
}

#[test]
fn normalize_mailbox_text_converts_fullwidth_digits_and_collapses_whitespace() {
    assert_eq!(
        normalize_mailbox_text("  OpenAI　验证码：４３８２１１ \n 下一步  "),
        "openai 验证码:438211 下一步"
    );
}

#[test]
fn validate_mailbox_binding_fields_requires_complete_pair() {
    assert!(validate_mailbox_binding_fields(None, None).is_ok());
    assert!(validate_mailbox_binding_fields(Some("session_1"), Some("mail@example.com")).is_ok());
    assert!(validate_mailbox_binding_fields(Some("session_1"), None).is_err());
    assert!(validate_mailbox_binding_fields(None, Some("mail@example.com")).is_err());
}

#[test]
fn normalize_mailbox_address_trims_and_lowercases() {
    assert_eq!(
        normalize_mailbox_address("  Mixed.Case+1@Example.COM "),
        Some("mixed.case+1@example.com".to_string())
    );
    assert_eq!(normalize_mailbox_address("   "), None);
}

#[test]
fn normalize_mailbox_domain_accepts_common_kaisoumail_variants() {
    assert_eq!(
        normalize_mailbox_domain("MAIL-TW.707079.XYZ"),
        Some("mail-tw.707079.xyz".to_string())
    );
    assert_eq!(
        normalize_mailbox_domain("@mail-tw.707079.xyz"),
        Some("mail-tw.707079.xyz".to_string())
    );
    assert_eq!(
        normalize_mailbox_domain("finance.lab.d5r@mail-tw.707079.xyz"),
        Some("mail-tw.707079.xyz".to_string())
    );
    assert_eq!(normalize_mailbox_domain("   "), None);
}

#[test]
fn kaisoumail_supported_domains_normalize_config_tokens() {
    let payload = KaisouMailMetaPayload {
        domains: vec![
            "707079.xyz".to_string(),
            "@707979.XYZ".to_string(),
            "finance.lab.d5r@fkoai.asia".to_string(),
        ],
    };
    let domains = kaisoumail_supported_domains(&payload);
    assert!(domains.contains("707079.xyz"));
    assert!(domains.contains("707979.xyz"));
    assert!(domains.contains("fkoai.asia"));
}

#[test]
fn validate_kaisoumail_mailbox_address_matches_requested_manual_address() {
    let payload = KaisouMailMailboxPayload {
        id: "mailbox_1".to_string(),
        address: "Finance.Lab.D5R@mail-tw.707079.xyz".to_string(),
        expires_at: None,
    };
    assert!(
        validate_kaisoumail_mailbox_address_matches_request(
            &payload,
            "finance.lab.d5r@mail-tw.707079.xyz"
        )
        .is_ok()
    );

    let rewritten = KaisouMailMailboxPayload {
        address: "other@mail-tw.707079.xyz".to_string(),
        ..payload
    };
    let err = validate_kaisoumail_mailbox_address_matches_request(
        &rewritten,
        "finance.lab.d5r@mail-tw.707079.xyz",
    )
    .expect_err("rewritten address should be rejected");
    assert!(err.to_string().contains("does not match requested"));
}

#[test]
fn requested_manual_mailbox_address_distinguishes_missing_from_blank_input() {
    assert!(matches!(
        requested_manual_mailbox_address(None),
        RequestedManualMailboxAddress::Missing
    ));
    assert_eq!(
        requested_manual_mailbox_address(Some("  Mixed.Case@Example.COM  ")),
        RequestedManualMailboxAddress::Valid("mixed.case@example.com".to_string())
    );
    assert_eq!(
        requested_manual_mailbox_address(Some("   ")),
        RequestedManualMailboxAddress::Invalid("   ".to_string())
    );
}

#[test]
fn mailbox_address_is_valid_rejects_broken_values() {
    assert!(mailbox_address_is_valid("valid.user@example.com"));
    assert!(!mailbox_address_is_valid("broken-address"));
    assert!(!mailbox_address_is_valid("missing-domain@"));
}

#[test]
fn mailbox_addresses_match_normalizes_case_and_whitespace() {
    assert!(mailbox_addresses_match(
        Some(" Manual.User@Example.com "),
        Some("manual.user@example.com")
    ));
    assert!(!mailbox_addresses_match(
        Some("one@example.com"),
        Some("two@example.com")
    ));
}

#[test]
fn normalize_mailbox_session_expires_at_converts_rfc3339_offsets_to_utc_iso() {
    assert_eq!(
        normalize_mailbox_session_expires_at(
            Some("2026-03-18T10:00:00+08:00"),
            Utc.with_ymd_and_hms(2026, 3, 17, 0, 0, 0).unwrap(),
        ),
        "2026-03-18T02:00:00Z"
    );
}

#[test]
fn normalize_mailbox_session_expires_at_falls_back_when_source_is_invalid() {
    let fallback = Utc.with_ymd_and_hms(2026, 3, 17, 8, 9, 10).unwrap();
    assert_eq!(
        normalize_mailbox_session_expires_at(Some("not-a-timestamp"), fallback),
        "2026-03-17T08:09:10Z"
    );
}

#[test]
fn expired_mailbox_session_requires_remote_delete_skips_attached_mailboxes() {
    let attached = OauthMailboxSessionRow {
        session_id: "session_attached".to_string(),
        remote_email_id: "email_attached".to_string(),
        email_address: "attached@example.com".to_string(),
        email_domain: "example.com".to_string(),
        mailbox_source: Some(OAUTH_MAILBOX_SOURCE_ATTACHED.to_string()),
        latest_code_value: None,
        latest_code_source: None,
        latest_code_updated_at: None,
        invite_subject: None,
        invite_copy_value: None,
        invite_copy_label: None,
        invite_updated_at: None,
        invited: 0,
        last_message_id: None,
        created_at: "2026-03-17T00:00:00Z".to_string(),
        updated_at: "2026-03-17T00:00:00Z".to_string(),
        expires_at: "2026-03-17T00:10:00Z".to_string(),
    };
    let generated = OauthMailboxSessionRow {
        mailbox_source: Some(OAUTH_MAILBOX_SOURCE_GENERATED.to_string()),
        ..attached.clone()
    };

    assert!(!expired_mailbox_session_requires_remote_delete(&attached));
    assert!(expired_mailbox_session_requires_remote_delete(&generated));
}

#[test]
fn kaisoumail_attach_status_is_not_readable_only_for_permission_and_missing() {
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
async fn create_oauth_mailbox_session_accepts_supported_domain_variants_for_existing_mailbox() {
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
async fn create_oauth_mailbox_session_lets_kaisoumail_generate_address_upstream() {
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
async fn create_oauth_mailbox_session_restores_expired_existing_mailbox_before_attach() {
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
async fn create_oauth_mailbox_session_creates_missing_supported_mailbox() {
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
async fn create_oauth_mailbox_session_rejects_true_unsupported_domains() {
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
async fn delete_oauth_mailbox_session_deletes_remote_for_generated_manual_mailbox() {
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
async fn cleanup_expired_oauth_mailbox_sessions_deletes_remote_for_generated_manual_mailbox() {
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
fn collect_unseen_mailbox_messages_stops_at_last_seen_id() {
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
fn collect_unseen_mailbox_messages_keeps_all_when_baseline_is_missing() {
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
fn next_mailbox_cursor_after_refresh_advances_to_latest_processed_message() {
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
fn next_mailbox_cursor_after_refresh_keeps_existing_cursor_when_nothing_was_processed() {
    let next = next_mailbox_cursor_after_refresh(Some("msg_3"), &[]);

    assert_eq!(next.as_deref(), Some("msg_3"));
}

#[test]
fn merge_mailbox_code_prefers_fresher_refresh_value() {
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
fn merge_mailbox_invite_keeps_newer_stored_value_when_refresh_is_older() {
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
