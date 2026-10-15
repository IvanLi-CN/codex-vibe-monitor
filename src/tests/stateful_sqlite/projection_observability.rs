use super::*;

fn prompt_cache_publication_timestamp(metrics: &ObservabilityRuntime) -> Option<f64> {
    metrics.render().lines().find_map(|line| {
        (line.starts_with("cvm_stats_projection_last_good_timestamp_seconds{")
            && line.contains("query=\"prompt_cache\""))
        .then(|| line.split_whitespace().last()?.parse().ok())
        .flatten()
    })
}

#[tokio::test]
async fn prompt_cache_publication_observation_requires_a_successful_baseline() {
    let mut state = crate::tests::test_state_with_openai_base(
        Url::parse("http://127.0.0.1:9").expect("fixture URL"),
    )
    .await;
    let metrics = ObservabilityRuntime::new(true);
    Arc::get_mut(&mut state)
        .expect("fixture state is exclusively owned")
        .observability = metrics.clone();
    let topic = SubscriptionTopic::PromptCacheWindow {
        selection: PromptCacheConversationSelection::Count(20),
        detail_level: PromptCacheConversationDetailLevel::Full,
        recent_invocation_limit: Some(16),
    };
    assert!(prompt_cache_publication_timestamp(&metrics).is_none());

    state
        .subscription_hub
        .refresh_topic(state.clone(), topic.clone(), false)
        .await
        .expect("publish a database-backed baseline");
    let published = prompt_cache_publication_timestamp(&metrics)
        .expect("a real baseline publication establishes freshness");
    assert!(published > 0.0);
    state
        .subscription_hub
        .prompt_cache_projection_health()
        .await;
    state
        .subscription_hub
        .prompt_cache_projection_health()
        .await;
    assert_eq!(
        prompt_cache_publication_timestamp(&metrics),
        Some(published)
    );

    state.subscription_hub.mark_topic_dirty(&topic).await;
    state.pool.close().await;
    assert!(
        state
            .subscription_hub
            .refresh_topic(state.clone(), topic, false)
            .await
            .is_err()
    );
    assert_eq!(
        prompt_cache_publication_timestamp(&metrics),
        Some(published),
        "a failed rebuild must preserve the last successful publication time"
    );
    state.shutdown.cancel();
}
