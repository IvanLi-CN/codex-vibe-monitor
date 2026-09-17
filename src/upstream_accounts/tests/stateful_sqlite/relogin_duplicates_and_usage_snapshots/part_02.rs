#[tokio::test]
pub(crate) async fn same_group_team_shared_org_accounts_keep_manual_mother_only() {
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
pub(crate) async fn new_oauth_accounts_with_shared_user_id_are_preserved_and_flagged() {
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
pub(crate) async fn legacy_shared_account_id_duplicates_ignore_mixed_plan_types() {
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
pub(crate) async fn shared_user_id_duplicates_ignore_mixed_plan_types() {
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
pub(crate) async fn latest_usage_sample_plan_type_restores_same_plan_duplicate_flags() {
    latest_usage_sample_plan_type_restores_same_plan_duplicate_flags_impl().await;
}

async fn latest_usage_sample_plan_type_restores_same_plan_duplicate_flags_impl() {
    let pool = test_pool().await;

    let inserted_ids = seed_legacy_plan_accounts(&pool).await;
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
pub(crate) async fn latest_usage_sample_plan_type_does_not_clear_legacy_shared_account_duplicates()
{
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
pub(crate) async fn update_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption()
{
    update_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption_impl().await;
}

async fn update_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let display_name = "Shared Mixed Plan";
    let shared_account_id = "shared_mixed_plan_account";
    let inserted_ids = seed_mixed_plan_accounts(
        &state.pool,
        display_name,
        shared_account_id,
        [
            ("shared-mixed-team@example.com", Some("team")),
            ("shared-mixed-pro@example.com", Some("pro")),
        ],
    )
    .await;

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
pub(crate) async fn update_account_preserves_system_tags_when_empty_tag_ids_are_sent() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Tag Preserve Target").await;
    ensure_account_has_gpt55_unsupported_tag(&state.pool, account_id)
        .await
        .expect("seed system tag");

    let original_tag_ids = sqlx::query_scalar::<_, i64>(
        r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load original account tags");
    assert!(!original_tag_ids.is_empty());

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: Some("preserved note".to_string()),
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
                tag_ids: Some(vec![]),
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("update should preserve system tags");

    assert_eq!(detail.note.as_deref(), Some("preserved note"));

    let updated_tag_ids = sqlx::query_scalar::<_, i64>(
        r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load updated account tags");
    assert_eq!(updated_tag_ids, original_tag_ids);
}

#[tokio::test]
pub(crate) async fn refresh_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption()
 {
    refresh_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption_impl().await;
}

async fn refresh_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption_impl() {
    let pool = test_pool().await;
    let crypto_key = derive_secret_key("refresh-mixed-plan-sample-fallback");
    let display_name = "Refresh Shared Mixed Plan";
    let shared_account_id = "refresh_shared_mixed_plan_account";
    let inserted_ids = seed_mixed_plan_accounts(
        &pool,
        display_name,
        shared_account_id,
        [
            ("refresh-mixed-team@example.com", Some("team")),
            ("refresh-mixed-pro@example.com", Some("pro")),
        ],
    )
    .await;

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
pub(crate) async fn unknown_plan_type_accounts_with_shared_account_id_remain_flagged() {
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
pub(crate) async fn persist_usage_snapshot_uses_explicit_effective_plan_type() {
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

use super::*;

async fn seed_legacy_plan_accounts(pool: &SqlitePool) -> Vec<i64> {
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
    inserted_ids
}

async fn seed_mixed_plan_accounts(
    pool: &SqlitePool,
    display_name: &str,
    shared_account_id: &str,
    accounts: [(&str, Option<&str>); 2],
) -> Vec<i64> {
    let mut inserted_ids = Vec::new();
    for (email, plan_type) in accounts {
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
    inserted_ids
}
