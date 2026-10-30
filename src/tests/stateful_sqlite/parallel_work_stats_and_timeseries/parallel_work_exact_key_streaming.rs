use super::*;

#[tokio::test]
async fn parallel_work_exact_keys_deduplicate_within_bucket_but_preserve_cross_bucket_keys() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let first_bucket = align_reporting_bucket_epoch(Utc::now().timestamp(), 3_600, Shanghai)
        .expect("align hour")
        - 7_200;
    let first_bucket_at = Utc
        .timestamp_opt(first_bucket, 0)
        .single()
        .expect("first bucket");
    let second_bucket_at = first_bucket_at + ChronoDuration::hours(1);
    for (invoke_id, occurred_at) in [
        (
            "parallel-exact-duplicate-a",
            first_bucket_at + ChronoDuration::minutes(5),
        ),
        (
            "parallel-exact-duplicate-b",
            first_bucket_at + ChronoDuration::minutes(10),
        ),
        (
            "parallel-exact-cross-bucket",
            second_bucket_at + ChronoDuration::minutes(5),
        ),
    ] {
        insert_parallel_work_invocation(&state.pool, invoke_id, occurred_at, "pck-shared").await;
    }

    let bucket_keys = query_parallel_work_exact_key_sets(
        &state.pool,
        first_bucket_at,
        second_bucket_at + ChronoDuration::hours(1),
        3_600,
        Shanghai,
        InvocationSourceScope::All,
        None,
        None,
        None,
    )
    .await
    .expect("query exact parallel-work bucket keys");
    assert_eq!(bucket_keys.get(&first_bucket).map(HashSet::len), Some(1));
    assert_eq!(
        bucket_keys.get(&(first_bucket + 3_600)).map(HashSet::len),
        Some(1)
    );
}
