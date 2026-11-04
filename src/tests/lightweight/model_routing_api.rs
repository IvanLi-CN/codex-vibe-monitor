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
fn model_routing_api_handles_missing_policy_and_invalid_model_ids() {
    let catalog = vec!["catalog-only".to_string()];
    let observed = vec!["observed-only".to_string(), "\t".to_string()];
    let oversized = "x".repeat(257);
    let policy = vec![oversized, "policy-only".to_string()];

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
