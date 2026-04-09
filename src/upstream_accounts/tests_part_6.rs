    #[tokio::test]
    async fn update_oauth_login_session_rejects_completed_relogin_repairs() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = insert_oauth_account(&state.pool, "Relogin Target").await;
        let relogin = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: None,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                account_id: Some(account_id),
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create relogin session")
        .0;

        let pending_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
            .await
            .expect("load relogin session")
            .expect("relogin session should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "relogin-access".to_string(),
                refresh_token: "relogin-refresh".to_string(),
                id_token: test_id_token(
                    "relogin@example.com",
                    Some("org_relogin"),
                    Some("user_relogin"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        let completed_account_id = persist_oauth_callback_inner(
            state.as_ref(),
            PersistOauthCallbackInput {
                display_name: "Relogin Target".to_string(),
                session: pending_session,
                claims: test_claims(
                    "relogin@example.com",
                    Some("org_relogin"),
                    Some("user_relogin"),
                ),
                encrypted_credentials,
                token_expires_at: "2026-04-01T00:00:00Z".to_string(),
            },
        )
        .await
        .expect("persist relogin callback");
        assert_eq!(completed_account_id, account_id);

        let completed_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
            .await
            .expect("load completed relogin session")
            .expect("completed relogin session should exist");
        assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
        assert_eq!(
            completed_session.updated_at,
            completed_session.consumed_at.clone().unwrap()
        );

        let mut repair_headers = HeaderMap::new();
        repair_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&relogin.updated_at).expect("valid updated_at header"),
        );
        let err =
            update_oauth_login_session(
                State(state.clone()),
                repair_headers,
                AxumPath(relogin.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Edited Relogin".to_string()),
                    group_name: OptionalField::Value("edited-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    group_node_shunt_enabled: OptionalField::Missing,
                    note: OptionalField::Value("edited note".to_string()),
                    group_note: OptionalField::Value("edited group note".to_string()),
                    concurrency_limit: OptionalField::Missing,
                    tag_ids: OptionalField::Value(vec![]),
                    is_mother: OptionalField::Value(true),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect_err("completed relogin repair should be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            err.1,
            "This login session can no longer be edited.".to_string()
        );

        let account = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load relogin target after rejected repair")
            .expect("relogin target should exist");
        assert_eq!(account.display_name, "Relogin Target");
        assert_ne!(account.group_name.as_deref(), Some("edited-group"));
        assert_ne!(account.note.as_deref(), Some("edited note"));
    }

    #[tokio::test]
    async fn upsert_oauth_account_preserves_route_cooldown_state_for_existing_account() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, "Cooldown OAuth Existing", None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Cooldown OAuth Existing",
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims(
                    "cooldown-existing@example.com",
                    Some("cooldown_org"),
                    Some("cooldown_user"),
                ),
                encrypted_credentials: "encrypted-original".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("insert oauth account");
        tx.commit().await.expect("commit insert tx");

        seed_route_cooldown(
            &pool,
            account_id,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            300,
        )
        .await;
        let before = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row before update")
            .expect("row exists before update");

        let mut tx = pool.begin().await.expect("begin update tx");
        let updated_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: Some(account_id),
                display_name: "Cooldown OAuth Existing",
                group_name: None,
                is_mother: false,
                note: Some("updated note".to_string()),
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims(
                    "cooldown-existing@example.com",
                    Some("cooldown_org"),
                    Some("cooldown_user"),
                ),
                encrypted_credentials: "encrypted-updated".to_string(),
                token_expires_at: "2026-03-15T00:00:00Z",
            },
        )
        .await
        .expect("update oauth account");
        tx.commit().await.expect("commit update tx");

        assert_eq!(updated_id, account_id);
        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row after update")
            .expect("row exists after update");

        assert_eq!(after.last_route_failure_at, before.last_route_failure_at);
        assert_eq!(
            after.last_route_failure_kind,
            before.last_route_failure_kind
        );
        assert_eq!(after.cooldown_until, before.cooldown_until);
        assert_eq!(
            after.consecutive_route_failures,
            before.consecutive_route_failures
        );
        assert_eq!(after.note.as_deref(), Some("updated note"));
        assert_eq!(
            after.encrypted_credentials.as_deref(),
            Some("encrypted-updated")
        );
    }

    #[tokio::test]
    async fn same_plan_type_accounts_with_shared_account_id_are_flagged_as_duplicates() {
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
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims("first@example.com", Some("org_shared"), Some("user_1")),
                encrypted_credentials: "encrypted-1".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
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
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims("second@example.com", Some("org_shared"), Some("user_2")),
                encrypted_credentials: "encrypted-2".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("second oauth insert");
        tx.commit().await.expect("commit tx 2");

        assert_ne!(first_id, second_id);
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pool_upstream_accounts WHERE kind = ?1",
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .fetch_one(&pool)
        .await
        .expect("count oauth rows");
        assert_eq!(count, 2);

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(matches!(
            duplicate_info
                .get(&first_id)
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptAccountId])
        ));
        assert!(matches!(
            duplicate_info
                .get(&second_id)
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptAccountId])
        ));

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .filter(|summary| summary.id == first_id || summary.id == second_id)
                .all(|summary| matches!(
                    summary
                        .duplicate_info
                        .as_ref()
                        .map(|info| info.reasons.as_slice()),
                    Some([DuplicateReason::SharedChatgptAccountId])
                ))
        );

        let first_detail = load_upstream_account_detail(&pool, first_id)
            .await
            .expect("load first detail")
            .expect("first detail exists");
        let second_detail = load_upstream_account_detail(&pool, second_id)
            .await
            .expect("load second detail")
            .expect("second detail exists");
        assert!(matches!(
            first_detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptAccountId])
        ));
        assert!(matches!(
            second_detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptAccountId])
        ));
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
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims(email, Some(account_id), Some("user_shared")),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
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

        let summaries = load_upstream_account_summaries(&pool)
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
    async fn mixed_plan_type_accounts_with_shared_account_id_are_not_flagged() {
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
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims_with_plan_type(email, Some("org_shared"), None, plan_type),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(duplicate_info.is_empty());

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .all(|summary| summary.duplicate_info.is_none())
        );

        for summary in summaries {
            let detail = load_upstream_account_detail(&pool, summary.id)
                .await
                .expect("load detail")
                .expect("detail exists");
            assert!(detail.summary.duplicate_info.is_none());
        }
    }

    #[tokio::test]
    async fn mixed_plan_type_accounts_with_shared_user_id_are_not_flagged() {
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
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(duplicate_info.is_empty());

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .all(|summary| summary.duplicate_info.is_none())
        );

        for summary in summaries {
            let detail = load_upstream_account_detail(&pool, summary.id)
                .await
                .expect("load detail")
                .expect("detail exists");
            assert!(detail.summary.duplicate_info.is_none());
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
                    token_expires_at: "2026-03-14T00:00:00Z",
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

        let summaries = load_upstream_account_summaries(&pool)
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
    async fn latest_usage_sample_plan_type_clears_mixed_plan_duplicate_flags() {
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
                    token_expires_at: "2026-03-14T00:00:00Z",
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
        assert!(duplicate_info.is_empty());
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
                    token_expires_at: "2026-03-14T00:00:00Z",
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
                token_expires_at: "2026-03-14T00:00:00Z",
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
                        refresh_token: "refresh-1".to_string(),
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
                token_expires_at: "2026-03-14T00:00:00Z",
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
                refresh_token: "refresh-2".to_string(),
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
                token_expires_at: "2026-03-14T00:00:00Z",
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
                token_expires_at: "2026-03-14T00:00:00Z",
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
                    token_expires_at: "2026-03-14T00:00:00Z",
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

        let summaries = load_upstream_account_summaries(&pool)
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
                    token_expires_at: "2026-03-14T00:00:00Z",
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

        let summaries = load_upstream_account_summaries(&pool)
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
                    token_expires_at: "2026-03-14T00:00:00Z",
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

        let summaries = load_upstream_account_summaries(&pool)
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
                    token_expires_at: "2026-03-14T00:00:00Z",
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

        let summaries = load_upstream_account_summaries(&pool)
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
    async fn relink_updates_existing_oauth_row_without_inserting() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.expect("begin tx");
        let original_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Original OAuth",
                group_name: Some("prod".to_string()),
                is_mother: false,
                note: Some("note".to_string()),
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims("first@example.com", Some("org_shared"), Some("user_1")),
                encrypted_credentials: "encrypted-1".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
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
                group_name: Some("prod".to_string()),
                is_mother: false,
                note: Some("fresh".to_string()),
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims("second@example.com", Some("org_shared"), Some("user_9")),
                encrypted_credentials: "encrypted-2".to_string(),
                token_expires_at: "2026-03-15T00:00:00Z",
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
    async fn display_name_uniqueness_is_case_insensitive_and_self_excluding() {
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
    fn parse_mailbox_code_prefers_subject_match() {
        let detail = MoeMailMessageDetail {
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
    fn parse_mailbox_code_falls_back_to_body_match() {
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
            id: "msg_3".to_string(),
            subject: Some("Alex has invited you to a workspace".to_string()),
            content: Some(
                "Join workspace: https://chatgpt.com/workspace/invite/abc123".to_string(),
            ),
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
            id: "msg_query_invite".to_string(),
            subject: Some("Alice has invited you to a workspace".to_string()),
            content: Some(
                "Open your invite: https://chatgpt.com/workspace?invite=abc123".to_string(),
            ),
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        let detail = MoeMailMessageDetail {
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
        assert!(
            validate_mailbox_binding_fields(Some("session_1"), Some("mail@example.com")).is_ok()
        );
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
    fn normalize_mailbox_domain_accepts_common_moemail_variants() {
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
    fn moemail_supported_domains_normalize_config_tokens() {
        let payload = MoeMailConfigPayload {
            email_domains: Some(
                "mail-tw.707079.xyz, @MAIL-US.707079.XYZ ; finance.lab.d5r@mail-eu.707079.xyz"
                    .to_string(),
            ),
        };
        let domains = moemail_supported_domains(&payload);
        assert!(domains.contains("mail-tw.707079.xyz"));
        assert!(domains.contains("mail-us.707079.xyz"));
        assert!(domains.contains("mail-eu.707079.xyz"));
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
    fn moemail_attach_status_is_not_readable_only_for_permission_and_missing() {
        assert!(moemail_attach_status_is_not_readable(
            reqwest::StatusCode::FORBIDDEN
        ));
        assert!(moemail_attach_status_is_not_readable(
            reqwest::StatusCode::NOT_FOUND
        ));
        assert!(!moemail_attach_status_is_not_readable(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(!moemail_attach_status_is_not_readable(
            reqwest::StatusCode::GATEWAY_TIMEOUT
        ));
    }

    #[tokio::test]
    async fn create_oauth_mailbox_session_accepts_supported_domain_variants_for_existing_mailbox() {
        let harness = spawn_moemail_test_harness(
            "@MAIL-TW.707079.XYZ, mail-us.707079.xyz",
            vec![(
                "email_existing".to_string(),
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
    async fn create_oauth_mailbox_session_creates_missing_supported_mailbox() {
        let harness = spawn_moemail_test_harness("@mail-tw.707079.xyz", Vec::new()).await;
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
        let harness = spawn_moemail_test_harness("mail-us.707079.xyz", Vec::new()).await;
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
        let harness = spawn_moemail_test_harness("@mail-tw.707079.xyz", Vec::new()).await;
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
        let harness = spawn_moemail_test_harness(
            "@mail-tw.707079.xyz",
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
            MoeMailMessageSummary {
                id: "msg_3".to_string(),
                subject: Some("newest".to_string()),
                received_at: Some("2026-03-16T03:00:00Z".to_string()),
            },
            MoeMailMessageSummary {
                id: "msg_2".to_string(),
                subject: Some("baseline".to_string()),
                received_at: Some("2026-03-16T02:00:00Z".to_string()),
            },
            MoeMailMessageSummary {
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
            MoeMailMessageSummary {
                id: "msg_2".to_string(),
                subject: None,
                received_at: Some("2026-03-16T02:00:00Z".to_string()),
            },
            MoeMailMessageSummary {
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
            MoeMailMessageSummary {
                id: "msg_5".to_string(),
                subject: Some("latest".to_string()),
                received_at: Some("2026-03-16T05:00:00Z".to_string()),
            },
            MoeMailMessageSummary {
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

    #[test]
    fn generate_mailbox_local_name_looks_like_human_or_org_style() {
        let local = generate_mailbox_local_name().expect("mailbox local part");
        assert!(local.len() >= 10);
        assert!(
            local
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' || ch == '-')
        );
        assert!(local.chars().any(|ch| ch.is_ascii_digit()));
        assert!(!local.starts_with('-'));
        assert!(!local.ends_with('-'));
    }

    #[test]
    fn random_base36_uses_letters_and_digits() {
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
    fn build_window_usage_range_aligns_to_current_reset_window() {
        let now = parse_rfc3339_utc("2026-03-30T12:30:00Z").expect("fixed now");
        let range = build_window_usage_range(now, 300, Some("2026-03-30T14:00:00Z"))
            .expect("aligned range");

        assert_eq!(
            range.start_at,
            parse_rfc3339_utc("2026-03-30T09:00:00Z").expect("expected start")
        );
        assert_eq!(range.end_at, now);
    }

    #[test]
    fn build_window_usage_range_reuses_stale_reset_window_bounds() {
        let now = parse_rfc3339_utc("2026-03-30T12:30:00Z").expect("fixed now");
        let range = build_window_usage_range(now, 300, Some("2026-03-29T23:00:00Z"))
            .expect("historical range");

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
    async fn enrich_window_actual_usage_for_summaries_counts_live_window_rows() {
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

        insert_window_actual_usage_invocation(
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
        insert_window_actual_usage_invocation(
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
        insert_window_actual_usage_invocation(
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
        insert_window_actual_usage_invocation(
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

        let mut summaries = load_upstream_account_summaries(&state.pool)
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
    async fn enrich_window_actual_usage_for_summaries_uses_matching_stale_reset_window() {
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

        insert_window_actual_usage_invocation(
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
        insert_window_actual_usage_invocation(
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
        insert_window_actual_usage_invocation(
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
    async fn enrich_window_actual_usage_for_summaries_reads_archived_rows_past_retention_cutoff() {
        let mut config =
            usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
        config.invocation_max_days = 1;
        config.archive_dir = PathBuf::from(format!(
            "target/archive-tests/window-actual-usage-{}",
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
        insert_window_actual_usage_invocation(
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
        seed_window_actual_usage_archive_batch(
            &state.pool,
            &state.config.archive_dir,
            "window-actual-usage-archive",
            &[(
                account_id,
                archived_row_at,
                Some(1200),
                Some(600),
                Some(200),
                Some(2000),
                Some(0.02),
            )],
        )
        .await;

        let mut items = vec![summary];
        enrich_window_actual_usage_for_summaries(state.as_ref(), &mut items)
            .await
            .expect("enrich actual usage with archive rows");

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
    async fn load_upstream_account_detail_with_actual_usage_serializes_actual_usage_camel_case() {
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
        insert_window_actual_usage_invocation(
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
        insert_window_actual_usage_invocation(
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
