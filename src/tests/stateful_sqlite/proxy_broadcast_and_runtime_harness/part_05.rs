#[tokio::test]
pub(crate) async fn forward_proxy_timeseries_preserves_retired_proxy_metadata() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1085".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
    let manual = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .cloned()
        .expect("manual node should exist");

    let bucket_start =
        align_bucket_epoch((Utc::now() - ChronoDuration::hours(5)).timestamp(), 3600, 0);
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual.key,
        Utc.timestamp_opt(bucket_start + 15 * 60, 0)
            .single()
            .expect("historical attempt timestamp should be valid"),
        true,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        ForwardProxyWeightBucket {
            proxy_key: &manual.key,
            bucket_start_epoch: bucket_start,
            sample_count: 1,
            min_weight: 0.72,
            max_weight: 0.72,
            avg_weight: 0.72,
            last_weight: 0.72,
        },
    )
    .await;

    let _ = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec![],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;

    let range_start = Utc
        .timestamp_opt(bucket_start, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket_start + 30 * 60, 0)
        .single()
        .expect("range end should be valid");
    let response = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: range_start,
            end: range_end,
            display_end: range_end,
            duration: range_end - range_start,
        },
    )
    .await
    .expect("forward proxy timeseries should succeed");

    let archived = response
        .nodes
        .iter()
        .find(|node| node.key == manual.key)
        .expect("retired proxy should remain queryable via historical metadata");
    assert_eq!(archived.display_name, manual.display_name);
    assert_eq!(archived.source, manual.source);
    assert_eq!(archived.endpoint_url, manual.endpoint_url);
}

#[tokio::test]
pub(crate) async fn reporting_tz_hour_alignment_rejects_sub_hour_dst_transition_windows() {
    let start = Utc
        .with_ymd_and_hms(2026, 10, 3, 15, 20, 0)
        .single()
        .expect("transition test start should be valid");
    let end = Utc
        .with_ymd_and_hms(2026, 10, 3, 15, 40, 0)
        .single()
        .expect("transition test end should be valid");

    assert!(!reporting_tz_has_whole_hour_offsets(
        chrono_tz::Australia::Lord_Howe,
        &RangeWindow {
            start,
            end,
            display_end: end,
            duration: end - start,
        }
    ));
}

#[tokio::test]
pub(crate) async fn parallel_work_day_all_fallbacks_when_requested_window_is_missing_in_sub_hour_zone()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 7, 12, 0, 0)
        .single()
        .expect("fixed now");
    assert!(should_fallback_parallel_work_day_all_window(
        "Asia/Kolkata".parse::<Tz>().expect("valid kolkata tz"),
        None,
        now,
    ));
    assert!(!should_fallback_parallel_work_day_all_window(
        "UTC".parse::<Tz>().expect("valid utc tz"),
        None,
        now,
    ));
}

#[test]
pub(crate) fn parallel_work_complete_window_preserves_local_hour_across_dst() {
    let reporting_tz = chrono_tz::America::New_York;
    let now = Utc
        .with_ymd_and_hms(2026, 4, 7, 5, 23, 0)
        .single()
        .expect("fixed now");

    let window =
        resolve_complete_parallel_work_window(now, ChronoDuration::days(30), 3_600, reporting_tz)
            .expect("resolve window");

    assert_eq!(
        window.end,
        Utc.with_ymd_and_hms(2026, 4, 7, 5, 0, 0)
            .single()
            .expect("fixed window end")
    );
    assert_eq!(
        window.start,
        Utc.with_ymd_and_hms(2026, 3, 8, 6, 0, 0)
            .single()
            .expect("fixed window start")
    );
    assert_eq!(
        window.end.with_timezone(&reporting_tz).time(),
        window.start.with_timezone(&reporting_tz).time()
    );
}

#[tokio::test]
pub(crate) async fn upsert_forward_proxy_weight_hourly_bucket_keeps_latest_sample_weight() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;
    let proxy_key = "manual://latest-sample-weight";
    let bucket_start_epoch = align_bucket_epoch(Utc::now().timestamp(), 3600, 0);

    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        0.90,
        1_000_000,
    )
    .await
    .expect("seed first weight sample");
    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        1.10,
        2_000_000,
    )
    .await
    .expect("seed second weight sample");
    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        0.70,
        1_500_000,
    )
    .await
    .expect("seed out-of-order weight sample");

    let row = sqlx::query(
        r#"
        SELECT
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            last_sample_epoch_us
        FROM forward_proxy_weight_hourly
        WHERE proxy_key = ?1 AND bucket_start_epoch = ?2
        "#,
    )
    .bind(proxy_key)
    .bind(bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("fetch aggregated weight bucket");

    assert_eq!(
        row.try_get::<i64, _>("sample_count").expect("sample_count"),
        3
    );
    assert!((row.try_get::<f64, _>("min_weight").expect("min_weight") - 0.70).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("max_weight").expect("max_weight") - 1.10).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("avg_weight").expect("avg_weight") - 0.90).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("last_weight").expect("last_weight") - 1.10).abs() < 1e-6);
    assert_eq!(
        row.try_get::<i64, _>("last_sample_epoch_us")
            .expect("last_sample_epoch_us"),
        2_000_000
    );
}

fn assert_initial_pricing_settings(initial: &SettingsResponse) {
    assert_eq!(initial.proxy.fast_mode_rewrite_mode, "disabled");
    for model in [
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "gpt-5.5",
        "gpt-5.5-pro",
    ] {
        assert!(initial.proxy.models.contains(&model.to_string()));
    }
    assert!(!initial.pricing.entries.is_empty());
    for model in [
        "gpt-5.2-codex",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "gpt-5.5",
        "gpt-5.5-pro",
        "gpt-5.4-mini",
    ] {
        assert!(
            initial
                .pricing
                .entries
                .iter()
                .any(|entry| entry.model == model),
            "pricing entry should include {model}"
        );
    }
}

#[tokio::test]
pub(crate) async fn pricing_settings_api_reads_and_persists_updates() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(initial) = get_settings(State(state.clone()))
        .await
        .expect("get settings should succeed");
    assert_initial_pricing_settings(&initial);

    let Json(updated) = put_pricing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "custom-ci".to_string(),
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

use super::*;
