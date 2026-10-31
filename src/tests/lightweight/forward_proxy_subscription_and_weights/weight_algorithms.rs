use super::*;

#[test]
fn forward_proxy_manager_keeps_one_positive_weight() {
    let mut manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );

    for runtime in manager.runtime.values_mut() {
        runtime.weight = -5.0;
    }
    manager.ensure_non_zero_weight();

    assert!(manager.runtime.values().any(|entry| entry.weight > 0.0));
}

#[test]
fn forward_proxy_algo_from_str_supports_v1_and_v2() {
    assert_eq!(
        ForwardProxyAlgo::from_str("v1").expect("v1 should parse"),
        ForwardProxyAlgo::V1
    );
    assert_eq!(
        ForwardProxyAlgo::from_str("V2").expect("v2 should parse"),
        ForwardProxyAlgo::V2
    );
    assert!(ForwardProxyAlgo::from_str("unexpected").is_err());
}

#[test]
fn forward_proxy_algo_config_defaults_to_latest_v2() {
    let algo = resolve_forward_proxy_algo_config(None, None).expect("default algo should resolve");
    assert_eq!(algo, ForwardProxyAlgo::V2);
}

#[test]
fn forward_proxy_algo_config_accepts_primary_env() {
    let algo =
        resolve_forward_proxy_algo_config(Some("v2"), None).expect("primary env should resolve");
    assert_eq!(algo, ForwardProxyAlgo::V2);
}

#[test]
fn forward_proxy_algo_config_rejects_legacy_env() {
    let err =
        resolve_forward_proxy_algo_config(None, Some("v1")).expect_err("legacy env should fail");
    assert_eq!(
        err.to_string(),
        "XY_FORWARD_PROXY_ALGO is not supported; rename it to FORWARD_PROXY_ALGO"
    );
}

#[test]
fn forward_proxy_algo_config_rejects_when_both_env_vars_are_set() {
    let err = resolve_forward_proxy_algo_config(Some("v2"), Some("v1"))
        .expect_err("legacy env should still win as a hard failure");
    assert_eq!(
        err.to_string(),
        "XY_FORWARD_PROXY_ALGO is not supported; rename it to FORWARD_PROXY_ALGO"
    );
}

#[test]
fn forward_proxy_manager_v2_keeps_two_positive_weights() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    for runtime in manager.runtime.values_mut() {
        runtime.weight = -5.0;
    }
    manager.ensure_non_zero_weight();

    let positive_count = manager
        .endpoints
        .iter()
        .filter_map(|endpoint| manager.runtime.get(&endpoint.key))
        .filter(|entry| entry.weight > 0.0)
        .count();
    assert_eq!(positive_count, 1);
}
