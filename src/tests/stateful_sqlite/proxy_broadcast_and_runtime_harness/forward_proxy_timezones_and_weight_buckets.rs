use super::*;

#[tokio::test]
async fn reporting_tz_hour_alignment_rejects_sub_hour_dst_transition_windows() {
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
async fn parallel_work_day_all_fallbacks_when_requested_window_is_missing_in_sub_hour_zone() {
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
fn parallel_work_complete_window_preserves_local_hour_across_dst() {
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
async fn upsert_forward_proxy_weight_hourly_bucket_keeps_latest_sample_weight() {
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
