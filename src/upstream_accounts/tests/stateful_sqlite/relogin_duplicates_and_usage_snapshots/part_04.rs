use super::*;

#[tokio::test]
#[ignore = "manual benchmark: seeds a prod-sized fixture and prints latency samples"]
pub(crate) async fn benchmark_upstream_account_roster_prod_sized() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_window_actual_usage_test_tables(&state.pool).await;
    let account_ids = seed_benchmark_roster(&state).await;

    let flat_query = || ListUpstreamAccountsQuery {
        page: Some(1),
        page_size: Some(20),
        ..Default::default()
    };
    let include_all_query = || ListUpstreamAccountsQuery {
        page: Some(1),
        page_size: Some(20),
        include_all: Some(true),
        ..Default::default()
    };

    let Json(flat_response) = list_upstream_accounts_from_params(state.clone(), flat_query())
        .await
        .expect("load flat roster for benchmark batch ids");
    let batch_account_ids = flat_response
        .items
        .iter()
        .map(|item| item.id)
        .take(20)
        .collect::<Vec<_>>();
    assert_eq!(account_ids.len(), 159);
    assert_eq!(batch_account_ids.len(), 20);

    run_roster_benchmark_queries(
        &state,
        &account_ids,
        &batch_account_ids,
        &flat_query,
        &include_all_query,
    )
    .await;
}

async fn seed_benchmark_roster(state: &Arc<AppState>) -> Vec<i64> {
    let group_names = (0..12)
        .map(|index| format!("benchmark-group-{index:02}"))
        .collect::<Vec<_>>();
    for group_name in &group_names {
        ensure_test_group_binding(&state.pool, group_name).await;
    }
    let captured_at = format_utc_iso(Utc::now());
    let now = Utc::now();
    let mut account_ids = Vec::with_capacity(159);
    for index in 0..159_usize {
        let display_name = format!("Benchmark Account {index:03}");
        let account_id = insert_api_key_account(&state.pool, &display_name).await;
        if index % 11 != 0 {
            let group_name = &group_names[index % group_names.len()];
            set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
        }
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            &captured_at,
            Some((10 + index % 65) as f64),
            Some((5 + index % 35) as f64),
        )
        .await;
        for hour_offset in 0..(24 * 7) {
            let occurred_at =
                shanghai_local_iso(now - ChronoDuration::hours(hour_offset as i64 + 1));
            let input_tokens = 900 + ((index + hour_offset) % 300) as i64;
            let output_tokens = 400 + ((index * 3 + hour_offset) % 180) as i64;
            let cache_input_tokens = 90 + ((index * 5 + hour_offset) % 70) as i64;
            insert_upstream_account_usage_hourly_row(
                &state.pool,
                UpstreamAccountUsageHourlyRow {
                    account_id,
                    occurred_at: &occurred_at,
                    request_count: 1 + ((index + hour_offset) % 3) as i64,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_cost: ((input_tokens + output_tokens + cache_input_tokens) as f64)
                        / 100_000.0,
                },
            )
            .await;
        }
        let live_tail_at = shanghai_local_iso(now - ChronoDuration::minutes((index % 45) as i64));
        insert_window_actual_usage_invocation!(
            &state.pool,
            account_id,
            &live_tail_at,
            Some(700 + (index % 120) as i64),
            Some(320 + (index % 90) as i64),
            Some(80 + (index % 30) as i64),
            Some(1100 + (index % 210) as i64),
            Some(0.011 + (index as f64 * 0.00001)),
        )
        .await;
        account_ids.push(account_id);
    }
    account_ids
}

async fn run_roster_benchmark_queries(
    state: &Arc<AppState>,
    account_ids: &[i64],
    batch_account_ids: &[i64],
    flat_query: &impl Fn() -> ListUpstreamAccountsQuery,
    include_all_query: &impl Fn() -> ListUpstreamAccountsQuery,
) {
    for _ in 0..5 {
        let _ = list_upstream_accounts_from_params(state.clone(), flat_query())
            .await
            .expect("warm flat roster");
        let _ = list_upstream_accounts_from_params(state.clone(), include_all_query())
            .await
            .expect("warm includeAll roster");
        let _ = get_upstream_account_window_usage(
            State(state.clone()),
            Json(UpstreamAccountWindowUsageRequest {
                account_ids: batch_account_ids.to_vec(),
            }),
        )
        .await
        .expect("warm usage batch");
    }
    let mut flat_samples_ms = Vec::with_capacity(30);
    let mut include_all_samples_ms = Vec::with_capacity(30);
    let mut window_usage_samples_ms = Vec::with_capacity(30);
    for _ in 0..30 {
        let started_at = std::time::Instant::now();
        let _ = list_upstream_accounts_from_params(state.clone(), flat_query())
            .await
            .expect("benchmark flat roster");
        flat_samples_ms.push(started_at.elapsed().as_secs_f64() * 1000.0);
        let started_at = std::time::Instant::now();
        let _ = list_upstream_accounts_from_params(state.clone(), include_all_query())
            .await
            .expect("benchmark includeAll roster");
        include_all_samples_ms.push(started_at.elapsed().as_secs_f64() * 1000.0);
        let started_at = std::time::Instant::now();
        let _ = get_upstream_account_window_usage(
            State(state.clone()),
            Json(UpstreamAccountWindowUsageRequest {
                account_ids: batch_account_ids.to_vec(),
            }),
        )
        .await
        .expect("benchmark window usage batch");
        window_usage_samples_ms.push(started_at.elapsed().as_secs_f64() * 1000.0);
    }
    let flat_p95 = benchmark_percentile(&flat_samples_ms, 0.95);
    let include_all_p95 = benchmark_percentile(&include_all_samples_ms, 0.95);
    let window_usage_p95 = benchmark_percentile(&window_usage_samples_ms, 0.95);
    println!(
        "benchmark_upstream_account_roster_prod_sized flat_page20 total_accounts={} samples={} avg_ms={:.2} p50_ms={:.2} p95_ms={:.2}",
        account_ids.len(),
        flat_samples_ms.len(),
        benchmark_average(&flat_samples_ms),
        benchmark_percentile(&flat_samples_ms, 0.50),
        flat_p95,
    );
    println!(
        "benchmark_upstream_account_roster_prod_sized include_all total_accounts={} samples={} avg_ms={:.2} p50_ms={:.2} p95_ms={:.2}",
        account_ids.len(),
        include_all_samples_ms.len(),
        benchmark_average(&include_all_samples_ms),
        benchmark_percentile(&include_all_samples_ms, 0.50),
        include_all_p95,
    );
    println!(
        "benchmark_upstream_account_roster_prod_sized window_usage_batch batch_size={} samples={} avg_ms={:.2} p50_ms={:.2} p95_ms={:.2}",
        batch_account_ids.len(),
        window_usage_samples_ms.len(),
        benchmark_average(&window_usage_samples_ms),
        benchmark_percentile(&window_usage_samples_ms, 0.50),
        window_usage_p95,
    );
    assert!(
        flat_p95 <= 100.0,
        "flat roster p95 exceeded budget: {flat_p95:.2}ms"
    );
    assert!(
        include_all_p95 <= 100.0,
        "includeAll roster p95 exceeded budget: {include_all_p95:.2}ms"
    );
    assert!(
        window_usage_p95 <= 100.0,
        "window usage batch p95 exceeded budget: {window_usage_p95:.2}ms"
    );
}
