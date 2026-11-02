use super::*;

#[tokio::test]
async fn stale_terminal_usage_refresh_leases_can_be_reclaimed() {
    for partial in [false, true] {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Stale Terminal Usage Lease").await;
        let first_lease = begin_upstream_account_refresh_attempt_with_lease(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
            "sync_maintenance",
        )
        .await
        .expect("initial usage claim")
        .expect("initial usage lease");

        if partial {
            assert!(
                record_upstream_account_refresh_failure_with_lease(
                    &pool,
                    account_id,
                    UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
                    "sync_maintenance",
                    "usage response contained a partial window set",
                    "usage_window_partial",
                    "额度窗口更新不完整，等待下一分钟重试。",
                    true,
                    None,
                    Some(&first_lease),
                )
                .await
                .expect("record partial usage result")
            );
        } else {
            assert!(
                complete_upstream_account_refresh_success_with_lease(
                    &pool,
                    account_id,
                    UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
                    "sync_maintenance",
                    None,
                    Some(&first_lease),
                )
                .await
                .expect("record successful usage result")
            );
        }

        assert!(
            begin_upstream_account_refresh_attempt_with_lease(
                &pool,
                account_id,
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
                "sync_maintenance",
            )
            .await
            .expect("reject a fresh terminal lease")
            .is_none()
        );

        sqlx::query(
            "UPDATE pool_upstream_account_refresh_state SET last_attempt_at = ?1 WHERE account_id = ?2 AND operation = ?3",
        )
        .bind(format_utc_iso(
            Utc::now() - ChronoDuration::seconds(UPSTREAM_ACCOUNT_REFRESH_RETRY_INTERVAL_SECS + 1),
        ))
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS)
        .execute(&pool)
        .await
        .expect("age terminal usage lease");

        let second_lease = begin_upstream_account_refresh_attempt_with_lease(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
            "sync_maintenance",
        )
        .await
        .expect("reclaim stale terminal usage lease")
        .expect("reclaimed usage lease");
        assert_ne!(first_lease, second_lease);
        assert!(
            release_upstream_account_refresh_lease(
                &pool,
                account_id,
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
                &second_lease,
            )
            .await
            .expect("release reclaimed usage lease")
        );
    }
}
