use super::*;

#[test]
fn model_routing_api_unions_catalog_policy_and_observed_models() {
    let catalog = vec![" gpt-5.5 ".to_string(), "catalog-only".to_string()];
    let policy = vec!["policy-only".to_string(), "gpt-5.5".to_string()];
    let observed = vec!["observed-only".to_string(), " ".to_string()];

    let models = model_routing_model_union(&catalog, Some(&policy), &observed);

    assert_eq!(
        models.into_iter().collect::<Vec<_>>(),
        vec![
            "catalog-only".to_string(),
            "gpt-5.5".to_string(),
            "observed-only".to_string(),
            "policy-only".to_string(),
        ]
    );
}

#[test]
fn model_routing_api_handles_missing_policy_and_empty_model_ids() {
    let catalog = vec!["catalog-only".to_string()];
    let observed = vec!["observed-only".to_string(), "\t".to_string()];
    let policy = vec!["policy-only".to_string()];

    let models = model_routing_model_union(&catalog, Some(&policy), &observed);

    assert_eq!(
        models.into_iter().collect::<Vec<_>>(),
        vec![
            "catalog-only".to_string(),
            "observed-only".to_string(),
            "policy-only".to_string(),
        ]
    );
    assert!(model_routing_model_union(&catalog, None, &[]).contains("catalog-only"));
}

#[test]
fn unknown_model_routing_state_does_not_claim_a_request_observation() {
    let state = unknown_model_routing_state(
        "catalog-only".to_string(),
        Some("2026-11-05T00:00:00Z".to_string()),
    );

    assert_eq!(state.state, "unknown");
    assert_eq!(state.last_seen_at, "2026-11-05T00:00:00Z");
    assert!(state.changed_at.is_none());
    assert_eq!(state.failure_count, 0);
}

#[test]
fn unknown_model_routing_state_marks_missing_observation_without_read_time() {
    let state = unknown_model_routing_state("policy-only".to_string(), None);

    assert_eq!(state.last_seen_at, "unknown");
    assert_ne!(state.last_seen_at, chrono::Utc::now().to_rfc3339());
}
