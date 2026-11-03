use super::*;

#[tokio::test]
async fn distinct_members_with_shared_account_id_are_not_flagged_as_duplicates() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx 1");
    ensure_display_name_available(&mut *tx, "First OAuth", None)
        .await
        .expect("first name available");
    let first_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "First OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims_with_plan_type(
                "first@example.com",
                Some("org_shared"),
                Some("user_1"),
                Some("k12"),
            ),
            encrypted_credentials: "encrypted-1".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("first oauth insert");
    tx.commit().await.expect("commit tx 1");

    let mut tx = pool.begin().await.expect("begin tx 2");
    ensure_display_name_available(&mut *tx, "Second OAuth", None)
        .await
        .expect("second name available");
    let second_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Second OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims_with_plan_type(
                "second@example.com",
                Some("org_shared"),
                Some("user_2"),
                Some("k12"),
            ),
            encrypted_credentials: "encrypted-2".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("second oauth insert");
    tx.commit().await.expect("commit tx 2");

    assert_ne!(first_id, second_id);
    let count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_accounts WHERE kind = ?1")
            .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
            .fetch_one(&pool)
            .await
            .expect("count oauth rows");
    assert_eq!(count, 2);

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(!duplicate_info.contains_key(&first_id));
    assert!(!duplicate_info.contains_key(&second_id));

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(
        summaries
            .iter()
            .filter(|summary| summary.id == first_id || summary.id == second_id)
            .all(|summary| summary.duplicate_info.is_none())
    );

    let first_detail = load_upstream_account_detail(&pool, first_id)
        .await
        .expect("load first detail")
        .expect("first detail exists");
    let second_detail = load_upstream_account_detail(&pool, second_id)
        .await
        .expect("load second detail")
        .expect("second detail exists");
    assert!(first_detail.summary.duplicate_info.is_none());
    assert!(second_detail.summary.duplicate_info.is_none());
}

#[tokio::test]
async fn load_duplicate_info_for_account_matches_global_duplicate_result() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx 1");
    ensure_display_name_available(&mut *tx, "Scoped Duplicate One", None)
        .await
        .expect("first name available");
    let first_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Scoped Duplicate One",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "scoped-1@example.com",
                Some("scoped_shared_org"),
                Some("scoped_user_shared"),
            ),
            encrypted_credentials: "encrypted-scoped-1".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("first oauth insert");
    tx.commit().await.expect("commit tx 1");

    let mut tx = pool.begin().await.expect("begin tx 2");
    ensure_display_name_available(&mut *tx, "Scoped Duplicate Two", None)
        .await
        .expect("second name available");
    let second_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Scoped Duplicate Two",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "scoped-2@example.com",
                Some("scoped_shared_org"),
                Some("scoped_user_shared"),
            ),
            encrypted_credentials: "encrypted-scoped-2".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("second oauth insert");
    tx.commit().await.expect("commit tx 2");

    let all_duplicates = load_duplicate_info_map(&pool)
        .await
        .expect("load all duplicate info");
    let first_scoped = load_duplicate_info_for_account(&pool, first_id)
        .await
        .expect("load scoped duplicate for first");
    let second_scoped = load_duplicate_info_for_account(&pool, second_id)
        .await
        .expect("load scoped duplicate for second");

    let first_all = all_duplicates
        .get(&first_id)
        .expect("first global duplicate");
    let first_scoped = first_scoped.expect("first scoped duplicate");
    assert_eq!(first_scoped.peer_account_ids, first_all.peer_account_ids);
    assert_eq!(first_scoped.reasons, first_all.reasons);

    let second_all = all_duplicates
        .get(&second_id)
        .expect("second global duplicate");
    let second_scoped = second_scoped.expect("second scoped duplicate");
    assert_eq!(second_scoped.peer_account_ids, second_all.peer_account_ids);
    assert_eq!(second_scoped.reasons, second_all.reasons);
}

#[tokio::test]
async fn same_group_team_shared_org_accounts_are_not_flagged_as_duplicates() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email, user_id) in [
        ("Team Member One", "team-1@example.com", "team_user_1"),
        ("Team Member Two", "team-2@example.com", "team_user_2"),
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
                group_name: Some("0414-3".to_string()),
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("shared_team_org"),
                    Some(user_id),
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

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(duplicate_info.is_empty());

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
            .all(|summary| summary.duplicate_info.is_none() && !summary.is_mother)
    );

    for account_id in inserted_ids {
        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert!(detail.summary.duplicate_info.is_none());
        assert!(!detail.summary.is_mother);
    }
}

#[tokio::test]
async fn same_group_team_shared_org_accounts_keep_manual_mother_only() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email, user_id, is_mother) in [
        (
            "Manual Mother One",
            "manual-mother-1@example.com",
            "manual_user_1",
            false,
        ),
        (
            "Manual Mother Two",
            "manual-mother-2@example.com",
            "manual_user_2",
            true,
        ),
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
                group_name: Some("shared-team-manual".to_string()),
                is_mother,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("shared_team_org_manual"),
                    Some(user_id),
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

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    let mother_ids = summaries
        .iter()
        .filter(|summary| inserted_ids.contains(&summary.id) && summary.is_mother)
        .map(|summary| summary.id)
        .collect::<Vec<_>>();
    assert_eq!(mother_ids, vec![inserted_ids[1]]);

    let first_detail = load_upstream_account_detail(&pool, inserted_ids[0])
        .await
        .expect("load first detail")
        .expect("first detail exists");
    let second_detail = load_upstream_account_detail(&pool, inserted_ids[1])
        .await
        .expect("load second detail")
        .expect("second detail exists");
    assert!(!first_detail.summary.is_mother);
    assert!(second_detail.summary.is_mother);
    assert!(first_detail.summary.duplicate_info.is_none());
    assert!(second_detail.summary.duplicate_info.is_none());
}

#[tokio::test]
async fn new_oauth_accounts_with_shared_user_id_are_preserved_and_flagged() {
    let pool = test_pool().await;

    for (display_name, email, account_id) in [
        ("First OAuth", "first@example.com", "org_1"),
        ("Second OAuth", "second@example.com", "org_2"),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        upsert_oauth_account(
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
                claims: &test_claims(email, Some(account_id), Some("user_shared")),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptUserId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(summaries.iter().all(|summary| matches!(
                    summary.duplicate_info.as_ref().map(|info| info.reasons.as_slice()),
                    Some([DuplicateReason::SharedChatgptUserId])
                )));

    for summary in summaries {
        let detail = load_upstream_account_detail(&pool, summary.id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert!(matches!(
            detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptUserId])
        ));
    }
}

#[tokio::test]
async fn legacy_shared_account_id_duplicates_ignore_mixed_plan_types() {
    let pool = test_pool().await;

    for (display_name, email, plan_type) in [
        ("Team OAuth", "team@example.com", Some("team")),
        ("Personal OAuth", "personal@example.com", Some("pro")),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        upsert_oauth_account(
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
                claims: &test_claims_with_plan_type(email, Some("org_shared"), None, plan_type),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
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
    assert!(summaries.iter().all(|summary| matches!(
                summary
                    .duplicate_info
                    .as_ref()
                    .map(|info| info.reasons.as_slice()),
                Some([DuplicateReason::SharedChatgptAccountId])
            )));

    for summary in summaries {
        let detail = load_upstream_account_detail(&pool, summary.id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert!(matches!(
            detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptAccountId])
        ));
    }
}

#[tokio::test]
async fn shared_user_id_duplicates_ignore_mixed_plan_types() {
    let pool = test_pool().await;

    for (display_name, email, account_id, plan_type) in [
        ("Team OAuth", "team@example.com", "org_team", Some("team")),
        (
            "Personal OAuth",
            "personal@example.com",
            "org_personal",
            Some("free"),
        ),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        upsert_oauth_account(
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
                    Some(account_id),
                    Some("user_shared"),
                    plan_type,
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
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptUserId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(summaries.iter().all(|summary| matches!(
                summary
                    .duplicate_info
                    .as_ref()
                    .map(|info| info.reasons.as_slice()),
                Some([DuplicateReason::SharedChatgptUserId])
            )));

    for summary in summaries {
        let detail = load_upstream_account_detail(&pool, summary.id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert!(matches!(
            detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptUserId])
        ));
    }
}

#[tokio::test]
async fn latest_usage_sample_plan_type_restores_same_plan_duplicate_flags() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email, plan_type) in [
        ("Legacy Team One", "legacy-team-1@example.com", None),
        ("Legacy Team Two", "legacy-team-2@example.com", Some("pro")),
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
                    Some("legacy_shared_org"),
                    None,
                    plan_type,
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

    for (index, account_id) in inserted_ids.iter().enumerate() {
        insert_limit_sample(
            &pool,
            *account_id,
            &format!("2026-03-15T00:00:0{}Z", index + 1),
            Some("team"),
        )
        .await;
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET plan_type_observed_at = '2026-03-14T00:00:00Z',
                    last_refreshed_at = '2026-03-14T00:00:00Z',
                    updated_at = '2026-03-14T00:00:00Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .execute(&pool)
        .await
        .expect("age account claims");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert_eq!(duplicate_info.len(), 2);
    assert!(
        duplicate_info
            .values()
            .all(|info| { info.reasons == vec![DuplicateReason::SharedChatgptAccountId] })
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
            .all(|summary| summary.plan_type.as_deref() == Some("team"))
    );
    assert!(
        summaries
            .iter()
            .filter(|summary| inserted_ids.contains(&summary.id))
            .all(|summary| matches!(
                summary
                    .duplicate_info
                    .as_ref()
                    .map(|info| info.reasons.as_slice()),
                Some([DuplicateReason::SharedChatgptAccountId])
            ))
    );

    for account_id in inserted_ids {
        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert_eq!(detail.summary.plan_type.as_deref(), Some("team"));
        assert!(matches!(
            detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptAccountId])
        ));
    }
}

#[tokio::test]
async fn latest_usage_sample_plan_type_does_not_clear_legacy_shared_account_duplicates() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email) in [
        ("Stale Team One", "stale-team-1@example.com"),
        ("Stale Team Two", "stale-team-2@example.com"),
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
                    Some("stale_shared_org"),
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

    insert_limit_sample(&pool, inserted_ids[0], "2026-03-15T00:00:01Z", Some("team")).await;
    insert_limit_sample(&pool, inserted_ids[1], "2026-03-15T00:00:02Z", Some("pro")).await;
    for account_id in &inserted_ids {
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET plan_type_observed_at = '2026-03-14T00:00:00Z',
                    last_refreshed_at = '2026-03-14T00:00:00Z',
                    updated_at = '2026-03-14T00:00:00Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .execute(&pool)
        .await
        .expect("age account claims");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
    );
}

#[tokio::test]
async fn update_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let display_name = "Shared Mixed Plan";
    let shared_account_id = "shared_mixed_plan_account";
    let mut inserted_ids = Vec::new();

    for (email, plan_type) in [
        ("shared-mixed-team@example.com", Some("team")),
        ("shared-mixed-pro@example.com", Some("pro")),
    ] {
        let mut tx = state.pool.begin().await.expect("begin tx");
        ensure_display_name_available_for_oauth_identity(
            &mut *tx,
            display_name,
            None,
            Some(shared_account_id),
            None,
            None,
            plan_type,
        )
        .await
        .expect("mixed-plan same-name create should be allowed");
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
                    Some(shared_account_id),
                    None,
                    plan_type,
                ),
                encrypted_credentials: format!("encrypted-{email}"),
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

    let current_account_id = inserted_ids[1];
    insert_limit_sample(
        &state.pool,
        current_account_id,
        "2026-03-15T00:00:02Z",
        Some("pro"),
    )
    .await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET plan_type = NULL,
                plan_type_observed_at = NULL,
                updated_at = '2026-03-15T00:00:03Z'
            WHERE id = ?1
            "#,
    )
    .bind(current_account_id)
    .execute(&state.pool)
    .await
    .expect("clear current account plan type");

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            current_account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: Some("mixed-plan note update".to_string()),
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("update should keep mixed-plan same-name exemption");

    assert_eq!(detail.summary.display_name, display_name);
    assert_eq!(detail.note.as_deref(), Some("mixed-plan note update"));
}

#[tokio::test]
async fn refresh_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption() {
    let pool = test_pool().await;
    let crypto_key = derive_secret_key("refresh-mixed-plan-sample-fallback");
    let display_name = "Refresh Shared Mixed Plan";
    let shared_account_id = "refresh_shared_mixed_plan_account";
    let mut inserted_ids = Vec::new();

    for (email, plan_type) in [
        ("refresh-mixed-team@example.com", Some("team")),
        ("refresh-mixed-pro@example.com", Some("pro")),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available_for_oauth_identity(
            &mut *tx,
            display_name,
            None,
            Some(shared_account_id),
            None,
            None,
            plan_type,
        )
        .await
        .expect("mixed-plan same-name create should be allowed");
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
                    Some(shared_account_id),
                    None,
                    plan_type,
                ),
                encrypted_credentials: format!("encrypted-{email}"),
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

    let current_account_id = inserted_ids[1];
    insert_limit_sample(
        &pool,
        current_account_id,
        "2026-03-15T00:00:02Z",
        Some("pro"),
    )
    .await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET plan_type = NULL,
                plan_type_observed_at = NULL,
                updated_at = '2026-03-15T00:00:03Z'
            WHERE id = ?1
            "#,
    )
    .bind(current_account_id)
    .execute(&pool)
    .await
    .expect("clear current account plan type");

    persist_oauth_credentials(
        &pool,
        current_account_id,
        &crypto_key,
        &StoredOauthCredentials {
            access_token: "refresh-access-2".to_string(),
            refresh_token: Some("refresh-token-2".to_string()),
            id_token: test_id_token(
                "refresh-mixed-pro@example.com",
                Some(shared_account_id),
                None,
                None,
            ),
            token_type: Some("Bearer".to_string()),
        },
        "2026-03-16T00:00:00Z",
    )
    .await
    .expect("refresh should keep mixed-plan same-name exemption");

    let row = load_upstream_account_row(&pool, current_account_id)
        .await
        .expect("load refreshed account")
        .expect("refreshed account exists");
    assert_eq!(row.display_name, display_name);
    assert_eq!(row.email.as_deref(), Some("refresh-mixed-pro@example.com"));
    assert!(row.last_refreshed_at.is_some());
}

#[tokio::test]
async fn unknown_plan_type_accounts_with_shared_account_id_remain_flagged() {
    let pool = test_pool().await;

    for (display_name, email, plan_type) in [
        ("Known Plan OAuth", "known@example.com", Some("team")),
        ("Unknown Plan OAuth", "unknown@example.com", None),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        upsert_oauth_account(
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
                    Some("unknown_plan_shared_org"),
                    None,
                    plan_type,
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
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
    );
}

#[tokio::test]
async fn persist_usage_snapshot_uses_explicit_effective_plan_type() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx");
    ensure_display_name_available(&mut *tx, "Snapshot OAuth", None)
        .await
        .expect("name available");
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Snapshot OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims_with_plan_type(
                "snapshot@example.com",
                Some("snapshot_org"),
                Some("snapshot_user"),
                Some("team"),
            ),
            encrypted_credentials: "encrypted-snapshot".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("oauth insert");
    tx.commit().await.expect("commit tx");

    let snapshot = NormalizedUsageSnapshot {
        plan_type: None,
        limit_id: "gpt-4".to_string(),
        limit_name: Some("GPT-4".to_string()),
        primary: None,
        secondary: None,
        credits: None,
        reserve: None,
        ..Default::default()
    };
    persist_usage_snapshot(&pool, account_id, Some("pro"), &snapshot, 30)
        .await
        .expect("persist snapshot");
    let stored_plan_type = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT plan_type
            FROM pool_upstream_account_limit_samples
            WHERE account_id = ?1
            ORDER BY captured_at DESC
            LIMIT 1
            "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load sample plan type");
    assert_eq!(stored_plan_type.as_deref(), Some("pro"));
}

#[tokio::test]
async fn persist_usage_snapshot_preserves_missing_window_from_latest_sample() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Partial Usage OAuth").await;
    let first_snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 18.0,
            window_duration_mins: 300,
            resets_at: Some("2026-03-17T14:00:00Z".to_string()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 42.0,
            window_duration_mins: 10_080,
            resets_at: Some("2026-03-18T00:00:00Z".to_string()),
        }),
        credits: Some(CreditsSnapshot {
            has_credits: false,
            unlimited: false,
            balance: Some("0".to_string()),
        }),
        reserve: None,
        ..Default::default()
    };
    persist_usage_snapshot(&pool, account_id, Some("team"), &first_snapshot, 30)
        .await
        .expect("persist complete usage snapshot");
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let partial_snapshot = NormalizedUsageSnapshot {
        primary: Some(NormalizedUsageWindow {
            used_percent: 24.0,
            window_duration_mins: 300,
            resets_at: Some("2026-03-17T15:00:00Z".to_string()),
        }),
        secondary: None,
        ..first_snapshot
    };
    persist_usage_snapshot(&pool, account_id, Some("team"), &partial_snapshot, 30)
        .await
        .expect("persist partial usage snapshot");

    let latest = load_latest_usage_sample(&pool, account_id)
        .await
        .expect("load latest usage sample")
        .expect("latest usage sample exists");
    assert_eq!(latest.primary_used_percent, Some(24.0));
    assert_eq!(latest.secondary_used_percent, Some(42.0));
    assert_eq!(latest.secondary_window_minutes, Some(10_080));
    assert_eq!(latest.credits_has_credits, Some(0));
    assert_eq!(latest.credits_unlimited, Some(0));
    assert_eq!(latest.credits_balance.as_deref(), Some("0"));
}

#[tokio::test]
async fn failed_usage_refresh_persists_unknown_reserve_without_erasing_normal_usage() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Reserve Failure OAuth").await;
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 32.0,
            window_duration_mins: 300,
            resets_at: Some("2026-03-17T14:00:00Z".to_string()),
        }),
        secondary: None,
        credits: None,
        reserve: Some(NormalizedReserveQuotaSnapshot {
            availability: RESERVE_AVAILABILITY_AVAILABLE.to_string(),
            allowed: Some(true),
            limit_reached: Some(false),
            normal_model_slug: Some("gpt-5.4-luna".to_string()),
            metered_feature: Some(RESERVE_METERED_FEATURE_BASE_MODEL_INFERENCE.to_string()),
            binding_source: Some(RESERVE_BINDING_SOURCE_USAGE.to_string()),
            primary: Some(NormalizedUsageWindow {
                used_percent: 10.0,
                window_duration_mins: 300,
                resets_at: Some("2026-03-17T14:00:00Z".to_string()),
            }),
            secondary: None,
            primary_available_percent: Some(90.0),
            secondary_available_percent: None,
        }),
        ..Default::default()
    };
    persist_usage_snapshot(&pool, account_id, Some("team"), &snapshot, 30)
        .await
        .expect("persist initial reserve snapshot");
    persist_unknown_reserve_usage_snapshot_with_lease(&pool, account_id, Some("team"), 30, None)
        .await
        .expect("persist unknown reserve snapshot");
    let latest = load_latest_usage_sample(&pool, account_id)
        .await
        .expect("load latest usage sample")
        .expect("latest usage sample exists");
    assert_eq!(latest.primary_used_percent, Some(32.0));
    assert_eq!(
        latest.reserve_status.as_deref(),
        Some(RESERVE_AVAILABILITY_UNKNOWN)
    );
    assert_eq!(latest.reserve_limit_reached, None);
    assert_eq!(latest.reserve_primary_used_percent, None);
    assert_eq!(latest.reserve_primary_available_percent, None);
}

#[tokio::test]
async fn refresh_without_plan_type_keeps_existing_plan_type_observed_at() {
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
async fn snapshot_plan_type_fallback_prefers_latest_effective_sample() {
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
        reserve: None,
        ..Default::default()
    };
    let effective_plan_type = resolve_snapshot_plan_type(&pool, &row, &snapshot)
        .await
        .expect("resolve snapshot plan type");
    assert_eq!(effective_plan_type.as_deref(), Some("pro"));
}

#[tokio::test]
async fn snapshot_plan_type_fallback_prefers_refreshed_claims_over_stale_non_empty_sample() {
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
        reserve: None,
        ..Default::default()
    };
    let effective_plan_type = resolve_snapshot_plan_type(&pool, &row, &snapshot)
        .await
        .expect("resolve snapshot plan type");
    assert_eq!(effective_plan_type.as_deref(), Some("pro"));
}

#[tokio::test]
async fn fresher_account_claims_override_stale_non_empty_samples() {
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
async fn refreshed_claims_override_older_non_empty_samples_without_newer_plan_samples() {
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
async fn same_second_refreshed_claims_win_against_latest_non_empty_sample() {
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
async fn metadata_updates_do_not_override_newer_usage_sample_plan_type() {
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
