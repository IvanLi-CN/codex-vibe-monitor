use super::*;

#[test]
fn terminal_observation_keeps_oldest_identity_until_commit_without_releasing_recovery_fence() {
    let registry = PendingIdentityRegistry::default();
    let first = "obsA000001";
    let second = "obsB000002";
    let occurred_at = "2026-10-15T00:00:00Z";
    registry.register(first, occurred_at, false);
    registry.register(second, occurred_at, false);
    let now = Instant::now();
    {
        let mut identities = registry.identities.lock().unwrap();
        identities
            .get_mut(&(first.to_owned(), occurred_at.to_owned(), false))
            .unwrap()
            .materialization_since = Some(now - Duration::from_secs(30));
        identities
            .get_mut(&(second.to_owned(), occurred_at.to_owned(), false))
            .unwrap()
            .materialization_since = Some(now - Duration::from_secs(10));
    }
    registry.register(first, occurred_at, false);
    let (count, age) = registry.materialization_snapshot();
    assert_eq!(count, 2);
    assert!(age >= Duration::from_secs(30));
    registry.observe_materialized(first, occurred_at, false);
    let (count, age) = registry.materialization_snapshot();
    assert_eq!(count, 1);
    assert!(age >= Duration::from_secs(10) && age < Duration::from_secs(20));
    assert_eq!(
        registry.identities.lock().unwrap().len(),
        2,
        "commit observation must not release journal recovery ownership"
    );
    registry.observe_materialized(second, occurred_at, false);
    assert_eq!(registry.materialization_snapshot(), (0, Duration::ZERO));
    registry.acknowledge(first, occurred_at, false);
    registry.acknowledge(second, occurred_at, false);
    assert!(registry.identities.lock().unwrap().is_empty());
}

#[test]
fn terminal_observation_does_not_count_running_attempts_or_reset_committed_duplicates() {
    let registry = PendingIdentityRegistry::default();
    assert_eq!(registry.materialization_snapshot(), (0, Duration::ZERO));
    registry.register("obsC000003", "fixture", true);
    assert_eq!(registry.materialization_snapshot().0, 1);
    registry.observe_materialized("obsC000003", "fixture", true);
    registry.register("obsC000003", "fixture", true);
    assert_eq!(registry.materialization_snapshot(), (0, Duration::ZERO));
}
