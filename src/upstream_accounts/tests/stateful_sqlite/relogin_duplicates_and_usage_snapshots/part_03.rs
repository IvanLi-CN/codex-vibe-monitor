#[tokio::test]
pub(crate) async fn refresh_without_plan_type_keeps_existing_plan_type_observed_at() {
    let pool = test_pool().await;
    let crypto_key = derive_secret_key("refresh-without-plan-type");

    let mut tx = pool.begin().await.expect("begin tx");
    ensure_display_name_available(&mut *tx, "Refresh OAuth", None)
        .await
        .expect("name available");
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Refresh OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims_with_plan_type(
                "refresh@example.com",
                Some("refresh_org"),
                Some("refresh_user"),
                Some("team"),
            ),
            encrypted_credentials: encrypt_credentials(
                &crypto_key,
                &StoredCredentials::Oauth(StoredOauthCredentials {
                    access_token: "access-1".to_string(),
                    refresh_token: Some("refresh-1".to_string()),
                    id_token: test_id_token(
                        "refresh@example.com",
                        Some("refresh_org"),
                        Some("refresh_user"),
                        Some("team"),
                    ),
                    token_type: Some("Bearer".to_string()),
                }),
            )
            .expect("encrypt oauth credentials"),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("oauth insert");
    tx.commit().await.expect("commit tx");

    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET plan_type_observed_at = '2026-03-15T00:00:01Z',
                last_refreshed_at = '2026-03-15T00:00:01Z',
                updated_at = '2026-03-15T00:00:01Z'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("seed observed_at");

    persist_oauth_credentials(
        &pool,
        account_id,
        &crypto_key,
        &StoredOauthCredentials {
            access_token: "access-2".to_string(),
            refresh_token: Some("refresh-2".to_string()),
            id_token: test_id_token(
                "refresh@example.com",
                Some("refresh_org"),
                Some("refresh_user"),
                None,
            ),
            token_type: Some("Bearer".to_string()),
        },
        "2026-03-16T00:00:00Z",
    )
    .await
    .expect("persist refreshed credentials");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row")
        .expect("row exists");
    assert_eq!(row.plan_type.as_deref(), Some("team"));
    assert_eq!(
        row.plan_type_observed_at.as_deref(),
        Some("2026-03-15T00:00:01Z")
    );
    assert!(row.last_refreshed_at.is_some());
    assert_ne!(
        row.last_refreshed_at.as_deref(),
        Some("2026-03-15T00:00:01Z")
    );
}

#[tokio::test]
pub(crate) async fn refresh_rejects_display_name_renames_that_conflict_with_other_accounts() {
    let pool = test_pool().await;
    let crypto_key = derive_secret_key("refresh-display-name-conflict");
    let account_id = insert_syncable_oauth_account(
        &pool,
        &crypto_key,
        "old@example.com",
        "old@example.com",
        "refresh_conflict_org",
        "refresh_conflict_user",
    )
    .await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET verified_email = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("old@example.com")
    .execute(&pool)
    .await
    .expect("seed verified email");
    let conflicting_id = insert_api_key_account(&pool, "renamed@example.com").await;

    let err = persist_oauth_credentials(
        &pool,
        account_id,
        &crypto_key,
        &StoredOauthCredentials {
            access_token: "access-2".to_string(),
            refresh_token: Some("refresh-2".to_string()),
            id_token: test_id_token(
                "renamed@example.com",
                Some("refresh_conflict_org"),
                Some("refresh_conflict_user"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        },
        "2026-03-16T00:00:00Z",
    )
    .await
    .expect_err("reject conflicting refresh rename");
    assert!(
        err.to_string().contains("displayName must be unique"),
        "unexpected error: {err:#}"
    );

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    assert_eq!(row.display_name, "old@example.com");
    assert_eq!(row.email.as_deref(), Some("old@example.com"));
    assert_eq!(row.verified_email.as_deref(), Some("old@example.com"));

    let conflict = load_upstream_account_row(&pool, conflicting_id)
        .await
        .expect("load conflicting row")
        .expect("conflicting row exists");
    assert_eq!(conflict.display_name, "renamed@example.com");
}

#[tokio::test]
pub(crate) async fn snapshot_plan_type_fallback_prefers_latest_effective_sample() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx");
    ensure_display_name_available(&mut *tx, "Fallback OAuth", None)
        .await
        .expect("name available");
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Fallback OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims_with_plan_type(
                "fallback@example.com",
                Some("fallback_org"),
                Some("fallback_user"),
                Some("team"),
            ),
            encrypted_credentials: "encrypted-fallback".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("oauth insert");
    tx.commit().await.expect("commit tx");

    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET plan_type = 'team',
                plan_type_observed_at = '2026-03-15T00:00:01Z',
                last_refreshed_at = '2026-03-15T00:00:01Z'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("age account claims");
    insert_limit_sample(&pool, account_id, "2026-03-15T00:00:02Z", Some("pro")).await;

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row")
        .expect("row exists");
    let snapshot = NormalizedUsageSnapshot {
        plan_type: None,
        limit_id: "gpt-4".to_string(),
        limit_name: Some("GPT-4".to_string()),
        primary: None,
        secondary: None,
        credits: None,
    };

    let effective_plan_type = resolve_snapshot_plan_type(&pool, &row, &snapshot)
        .await
        .expect("resolve snapshot plan type");
    assert_eq!(effective_plan_type.as_deref(), Some("pro"));
}

#[tokio::test]
pub(crate) async fn snapshot_plan_type_fallback_prefers_refreshed_claims_over_stale_non_empty_sample()
 {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx");
    ensure_display_name_available(&mut *tx, "Refreshed Fallback OAuth", None)
        .await
        .expect("name available");
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Refreshed Fallback OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims_with_plan_type(
                "refreshed-fallback@example.com",
                Some("refreshed_fallback_org"),
                Some("refreshed_fallback_user"),
                Some("team"),
            ),
            encrypted_credentials: "encrypted-refreshed-fallback".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("oauth insert");
    tx.commit().await.expect("commit tx");

    insert_limit_sample(&pool, account_id, "2026-03-15T00:00:01Z", Some("team")).await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET plan_type = 'pro',
                plan_type_observed_at = '2026-03-15T00:00:02Z',
                last_refreshed_at = '2026-03-15T00:00:02Z'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("refresh account claims");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row")
        .expect("row exists");
    let snapshot = NormalizedUsageSnapshot {
        plan_type: None,
        limit_id: "gpt-4".to_string(),
        limit_name: Some("GPT-4".to_string()),
        primary: None,
        secondary: None,
        credits: None,
    };

    let effective_plan_type = resolve_snapshot_plan_type(&pool, &row, &snapshot)
        .await
        .expect("resolve snapshot plan type");
    assert_eq!(effective_plan_type.as_deref(), Some("pro"));
}

#[tokio::test]
pub(crate) async fn fresher_account_claims_override_stale_non_empty_samples() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email) in [
        ("Refreshed Team One", "refreshed-team-1@example.com"),
        ("Refreshed Team Two", "refreshed-team-2@example.com"),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("refreshed_shared_org"),
                    None,
                    Some("team"),
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }

    for account_id in &inserted_ids {
        insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:01Z", Some("team")).await;
        insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:02Z", None).await;
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET plan_type = 'pro',
                    plan_type_observed_at = '2026-03-15T00:00:03Z',
                    last_refreshed_at = '2026-03-15T00:00:03Z',
                    updated_at = '2026-03-15T00:00:03Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .execute(&pool)
        .await
        .expect("refresh account claims");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(
        summaries
            .iter()
            .filter(|summary| inserted_ids.contains(&summary.id))
            .all(|summary| summary.plan_type.as_deref() == Some("pro"))
    );

    for account_id in inserted_ids {
        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert_eq!(detail.summary.plan_type.as_deref(), Some("pro"));
    }
}

#[tokio::test]
pub(crate) async fn refreshed_claims_override_older_non_empty_samples_without_newer_plan_samples() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email) in [
        ("Claims Fresh One", "claims-fresh-1@example.com"),
        ("Claims Fresh Two", "claims-fresh-2@example.com"),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("claims_fresh_shared_org"),
                    None,
                    Some("team"),
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }

    for account_id in &inserted_ids {
        insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:01Z", Some("team")).await;
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET plan_type = 'pro',
                    plan_type_observed_at = '2026-03-15T00:00:02Z',
                    last_refreshed_at = '2026-03-15T00:00:02Z',
                    updated_at = '2026-03-15T00:00:03Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .execute(&pool)
        .await
        .expect("refresh account claims");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(
        summaries
            .iter()
            .filter(|summary| inserted_ids.contains(&summary.id))
            .all(|summary| summary.plan_type.as_deref() == Some("pro"))
    );
}

#[tokio::test]
pub(crate) async fn same_second_refreshed_claims_win_against_latest_non_empty_sample() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email) in [
        ("Same Second One", "same-second-1@example.com"),
        ("Same Second Two", "same-second-2@example.com"),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("same_second_org"),
                    None,
                    Some("team"),
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }

    for account_id in &inserted_ids {
        insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:02Z", Some("team")).await;
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET plan_type = 'pro',
                    plan_type_observed_at = '2026-03-15T00:00:02Z',
                    last_refreshed_at = '2026-03-15T00:00:02Z',
                    updated_at = '2026-03-15T00:00:02Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .execute(&pool)
        .await
        .expect("seed same-second claims");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(
        summaries
            .iter()
            .filter(|summary| inserted_ids.contains(&summary.id))
            .all(|summary| summary.plan_type.as_deref() == Some("pro"))
    );
}

#[tokio::test]
pub(crate) async fn metadata_updates_do_not_override_newer_usage_sample_plan_type() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email) in [
        ("Sample Fresh One", "sample-fresh-1@example.com"),
        ("Sample Fresh Two", "sample-fresh-2@example.com"),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("sample_fresh_shared_org"),
                    None,
                    Some("team"),
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }

    for account_id in &inserted_ids {
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET plan_type = 'team',
                    plan_type_observed_at = '2026-03-15T00:00:01Z',
                    last_refreshed_at = '2026-03-15T00:00:01Z',
                    updated_at = '2026-03-15T00:00:01Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .execute(&pool)
        .await
        .expect("seed account claims");
        insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:02Z", Some("pro")).await;
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    updated_at = '2026-03-15T00:00:03Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .execute(&pool)
        .await
        .expect("simulate metadata update");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(
        summaries
            .iter()
            .filter(|summary| inserted_ids.contains(&summary.id))
            .all(|summary| summary.plan_type.as_deref() == Some("pro"))
    );
}

#[tokio::test]
pub(crate) async fn relink_updates_existing_oauth_row_without_inserting() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx");
    let original_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Original OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: Some("prod".to_string()),
            is_mother: false,
            note: Some("note".to_string()),
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims("first@example.com", Some("org_shared"), Some("user_1")),
            encrypted_credentials: "encrypted-1".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("insert original oauth");
    tx.commit().await.expect("commit tx");

    let mut tx = pool.begin().await.expect("begin relink tx");
    ensure_display_name_available(&mut *tx, "Renamed OAuth", Some(original_id))
        .await
        .expect("name available");
    let relinked_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: Some(original_id),
            display_name: "Renamed OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: Some("prod".to_string()),
            is_mother: false,
            note: Some("fresh".to_string()),
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims("second@example.com", Some("org_shared"), Some("user_9")),
            encrypted_credentials: "encrypted-2".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-15T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("relink oauth");
    tx.commit().await.expect("commit relink tx");

    assert_eq!(relinked_id, original_id);
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_accounts")
        .fetch_one(&pool)
        .await
        .expect("count accounts");
    assert_eq!(count, 1);

    let renamed = load_upstream_account_row(&pool, original_id)
        .await
        .expect("load updated row")
        .expect("row exists");
    assert_eq!(renamed.display_name, "Renamed OAuth");
    assert_eq!(renamed.chatgpt_user_id.as_deref(), Some("user_9"));
}

#[tokio::test]
pub(crate) async fn display_name_uniqueness_is_case_insensitive_and_self_excluding() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, " Alpha ").await;

    let mut tx = pool.begin().await.expect("begin tx conflict");
    let conflict = ensure_display_name_available(&mut *tx, "alpha", None).await;
    assert_eq!(
        conflict,
        Err((
            StatusCode::CONFLICT,
            "displayName must be unique".to_string()
        ))
    );

    let allowed = ensure_display_name_available(&mut *tx, " alpha ", Some(account_id)).await;
    assert!(allowed.is_ok());
}

#[test]
pub(crate) fn parse_mailbox_code_prefers_subject_match() {
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
pub(crate) fn decode_mailbox_detail_accepts_text_and_preview_text_together() {
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
pub(crate) fn kaisoumail_config_debug_redacts_api_key() {
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
pub(crate) fn parse_mailbox_code_falls_back_to_body_match() {
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
pub(crate) fn parse_mailbox_code_supports_localized_subjects() {
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
pub(crate) fn parse_mailbox_code_supports_localized_html_and_fullwidth_digits() {
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
pub(crate) fn parse_mailbox_code_prefers_digits_after_marker() {
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
pub(crate) fn parse_mailbox_code_rejects_weak_subject_match_without_local_brand() {
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
pub(crate) fn parse_mailbox_code_rejects_strong_subject_match_without_brand() {
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
pub(crate) fn parse_mailbox_code_rejects_unrelated_numbers_without_code_semantics() {
    let detail = KaisouMailMessageDetail {
        id: "msg_negative_code".to_string(),
        subject: Some("OpenAI receipt 438211".to_string()),
        content: Some("Invoice total: 23.00 USD".to_string()),
        html: None,
        received_at: Some("2026-03-24T00:05:00Z".to_string()),
    };

    assert!(parse_mailbox_code(&detail).is_none());
}

use super::*;
