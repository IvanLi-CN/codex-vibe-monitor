use super::*;

#[tokio::test]
async fn pricing_settings_api_reads_and_persists_updates() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(initial) = get_settings(State(state.clone()))
        .await
        .expect("get settings should succeed");
    assert_eq!(initial.proxy.fast_mode_rewrite_mode, "disabled");
    assert!(initial.proxy.models.contains(&"gpt-5.6-sol".to_string()));
    assert!(initial.proxy.models.contains(&"gpt-5.6-terra".to_string()));
    assert!(initial.proxy.models.contains(&"gpt-5.6-luna".to_string()));
    assert!(initial.proxy.models.contains(&"gpt-5.5".to_string()));
    assert!(initial.proxy.models.contains(&"gpt-5.5-pro".to_string()));
    assert!(!initial.pricing.entries.is_empty());
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.2-codex")
    );
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.6-sol")
    );
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.6-terra")
    );
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.6-luna")
    );
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.5")
    );
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.5-pro")
    );
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.4-mini")
    );

    let Json(updated) = put_pricing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "custom-ci".to_string(),
            quotes: None,
            entries: vec![PricingEntry {
                model: "gpt-5.2-codex".to_string(),
                input_per_1m: 8.8,
                output_per_1m: 18.8,
                cache_input_per_1m: Some(0.88),
                cache_read_per_1m: Some(0.88),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            }],
        }),
    )
    .await
    .expect("put pricing settings should succeed");

    assert_eq!(updated.catalog_version, "custom-ci");
    assert_eq!(updated.entries.len(), 1);
    assert_eq!(updated.entries[0].model, "gpt-5.2-codex");
    assert_eq!(updated.entries[0].input_per_1m, 8.8);
    assert_eq!(updated.entries[0].cache_input_per_1m, Some(0.88));
    assert_eq!(updated.entries[0].cache_read_per_1m, Some(0.88));
    assert_eq!(updated.entries[0].cache_write_per_1m, None);

    let persisted = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing settings should persist");
    assert_eq!(persisted.version, "custom-ci");
    assert_eq!(persisted.models.len(), 1);
    let pricing = persisted
        .models
        .get("gpt-5.2-codex")
        .expect("gpt-5.2-codex should persist");
    assert_eq!(pricing.input_per_1m, 8.8);
    assert_eq!(pricing.output_per_1m, 18.8);
}

#[tokio::test]
async fn normal_http_terminal_persistence_does_not_emit_retired_websocket_state() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    persist_and_broadcast_proxy_capture_terminal_record(
        &state,
        test_proxy_capture_record("normal-http-terminal", "2026-09-28 15:00:00"),
    )
    .await
    .expect("persist normal HTTP terminal record");

    let retired_invocation_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE json_valid(payload)
          AND (
                json_extract(payload, '$.transport') = 'websocket'
                OR json_type(payload, '$.streamTerminalEvent') IS NOT NULL
          )
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count retired WebSocket invocation state");
    assert_eq!(retired_invocation_count, 0);

    let retired_tag_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_tags WHERE system_key = 'unsupported_transport:websocket'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count retired WebSocket tags");
    assert_eq!(retired_tag_count, 0);

    let retired_tag_link_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_account_tags account_tag
        JOIN pool_tags tag ON tag.id = account_tag.tag_id
        WHERE tag.system_key = 'unsupported_transport:websocket'
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count retired WebSocket tag links");
    assert_eq!(retired_tag_link_count, 0);
}
