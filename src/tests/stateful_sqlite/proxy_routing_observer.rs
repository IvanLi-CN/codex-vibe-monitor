use super::*;
use crate::upstream_accounts::{
    invalidate_pool_routing_runtime_cache, load_pool_routing_runtime_cache_with_status,
};

#[tokio::test]
async fn routing_cache_observation_preserves_all_business_pool_capacity() {
    let state = test_state_from_config(test_config(), true).await;
    crate::upstream_accounts::load_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("initialize routing cache and observer");

    // A warmed routing observer must leave every configured business slot usable.
    let mut leases = Vec::new();
    for _ in 0..state.pool.options().get_max_connections() {
        leases.push(
            tokio::time::timeout(Duration::from_secs(2), state.pool.acquire())
                .await
                .expect("routing observation must not reserve business pool capacity")
                .expect("acquire configured business connection"),
        );
    }
    drop(leases);
    state.shutdown.cancel();
}

#[tokio::test]
async fn routing_cache_observer_detects_independent_fixture_writes() {
    let state = test_state_from_config(test_config(), true).await;
    sqlx::query("UPDATE pool_routing_settings SET cache_hit_protection_enabled = 0 WHERE id = 1")
        .execute(&state.pool)
        .await
        .expect("seed disabled protection");
    let (initial, cache_hit) = load_pool_routing_runtime_cache_with_status(state.as_ref())
        .await
        .expect("initialize cache and observer");
    assert!(!cache_hit);
    assert!(!initial.cache_hit_protection.enabled);
    let mut generation = initial.generation;

    for enabled in [true, false] {
        let (_, cache_hit) = load_pool_routing_runtime_cache_with_status(state.as_ref())
            .await
            .expect("read unchanged cache");
        assert!(cache_hit, "unchanged database keeps the warm snapshot");
        sqlx::query(
            "UPDATE pool_routing_settings SET cache_hit_protection_enabled = ? WHERE id = 1",
        )
        .bind(i64::from(enabled))
        .execute(&state.pool)
        .await
        .expect("commit an independent fixture write");
        let (changed, cache_hit) = load_pool_routing_runtime_cache_with_status(state.as_ref())
            .await
            .expect("observe updated routing settings");
        assert!(
            !cache_hit,
            "fixture write must invalidate the warm snapshot"
        );
        assert_eq!(changed.cache_hit_protection.enabled, enabled);
        assert!(changed.generation > generation);
        generation = changed.generation;
    }
    state.shutdown.cancel();
}

#[tokio::test]
async fn routing_cache_explicit_invalidation_advances_generation_without_database_change() {
    let state = test_state_from_config(test_config(), true).await;
    let (initial, _) = load_pool_routing_runtime_cache_with_status(state.as_ref())
        .await
        .expect("initialize routing snapshot");
    let (warm, cache_hit) = load_pool_routing_runtime_cache_with_status(state.as_ref())
        .await
        .expect("reuse unchanged routing snapshot");
    assert!(cache_hit);
    assert_eq!(warm.generation, initial.generation);

    invalidate_pool_routing_runtime_cache(state.as_ref()).await;
    let (refreshed, cache_hit) = load_pool_routing_runtime_cache_with_status(state.as_ref())
        .await
        .expect("refresh explicitly invalidated routing snapshot");
    assert!(!cache_hit);
    assert!(refreshed.generation > initial.generation);
    assert!(refreshed.model_routing.generation > initial.model_routing.generation);
    assert_eq!(
        refreshed.cache_hit_protection.enabled,
        initial.cache_hit_protection.enabled
    );
    state.shutdown.cancel();
}
