pub(crate) fn query_flag_true(query: &HashMap<String, String>, key: &str) -> bool {
    query.get(key).is_some_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardProxyAttemptWindowStats {
    pub(crate) attempts: i64,
    pub(crate) success_count: i64,
    pub(crate) avg_latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyWindowStatsResponse {
    pub(crate) attempts: i64,
    pub(crate) success_rate: Option<f64>,
    pub(crate) avg_latency_ms: Option<f64>,
}

impl From<ForwardProxyAttemptWindowStats> for ForwardProxyWindowStatsResponse {
    fn from(value: ForwardProxyAttemptWindowStats) -> Self {
        let success_rate = if value.attempts > 0 {
            Some((value.success_count as f64) / (value.attempts as f64))
        } else {
            None
        };
        Self {
            attempts: value.attempts,
            success_rate,
            avg_latency_ms: value.avg_latency_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyStatsResponse {
    pub(crate) one_minute: ForwardProxyWindowStatsResponse,
    pub(crate) fifteen_minutes: ForwardProxyWindowStatsResponse,
    pub(crate) one_hour: ForwardProxyWindowStatsResponse,
    pub(crate) one_day: ForwardProxyWindowStatsResponse,
    pub(crate) seven_days: ForwardProxyWindowStatsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) stats: ForwardProxyStatsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyBindingNodeResponse {
    pub(crate) key: String,
    pub(crate) alias_keys: Vec<String>,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) protocol_label: String,
    pub(crate) egress_ip: Option<String>,
    pub(crate) egress_ip_checked_at: Option<String>,
    pub(crate) egress_ip_provider: Option<String>,
    pub(crate) egress_ip_error: Option<String>,
    pub(crate) egress_ip_error_at: Option<String>,
    pub(crate) penalized: bool,
    pub(crate) selectable: bool,
    pub(crate) last24h: Vec<ForwardProxyHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxySettingsResponse {
    pub(crate) proxy_urls: Vec<String>,
    pub(crate) subscription_urls: Vec<String>,
    pub(crate) subscription_update_interval_secs: u64,
    pub(crate) nodes: Vec<ForwardProxyNodeResponse>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardProxyHourlyStatsPoint {
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyWeightHourlyStatsPoint {
    pub(crate) sample_count: i64,
    pub(crate) min_weight: f64,
    pub(crate) max_weight: f64,
    pub(crate) avg_weight: f64,
    pub(crate) last_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyHourlyBucketResponse {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyWeightHourlyBucketResponse {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) sample_count: i64,
    pub(crate) min_weight: f64,
    pub(crate) max_weight: f64,
    pub(crate) avg_weight: f64,
    pub(crate) last_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLiveNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) stats: ForwardProxyStatsResponse,
    pub(crate) last24h: Vec<ForwardProxyHourlyBucketResponse>,
    pub(crate) weight24h: Vec<ForwardProxyWeightHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLiveStatsResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) nodes: Vec<ForwardProxyLiveNodeResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyTimeseriesNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) buckets: Vec<ForwardProxyHourlyBucketResponse>,
    pub(crate) weight_buckets: Vec<ForwardProxyWeightHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyTimeseriesResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) effective_bucket: String,
    pub(crate) available_buckets: Vec<String>,
    pub(crate) nodes: Vec<ForwardProxyTimeseriesNodeResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_with_manual_proxy() -> ForwardProxyManager {
        ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec!["http://jp-edge-01:8080".to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        )
    }

    fn current_binding_node(manager: &ForwardProxyManager) -> ForwardProxyBindingNodeResponse {
        manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .expect("missing non-direct binding node")
    }

    #[test]
    fn binding_nodes_include_selectable_direct_with_protocol_label() {
        let manager = manager_with_manual_proxy();

        assert!(!manager.runtime.contains_key(FORWARD_PROXY_DIRECT_KEY));

        let direct = manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
            .expect("missing direct binding node");

        assert_eq!(direct.display_name, FORWARD_PROXY_DIRECT_LABEL);
        assert_eq!(direct.protocol_label, "DIRECT");
        assert!(direct.selectable);
        assert!(!direct.penalized);
    }

    #[test]
    fn binding_nodes_use_name_driven_binding_keys_and_keep_runtime_aliases() {
        let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京节点";
        let normalized_proxy_url =
            normalize_share_link_scheme(proxy_url, "vless").expect("normalize vless url");
        let legacy_alias = {
            let parsed = Url::parse(&normalized_proxy_url).expect("parse normalized vless url");
            stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
        };
        let runtime_key = normalize_single_proxy_key(proxy_url).expect("canonical vless key");
        let binding_key = forward_proxy_binding_key_candidates(
            &forward_proxy_binding_parts_from_raw(proxy_url, None)
                .expect("binding parts from vless url"),
        )[0]
        .clone();
        assert_ne!(binding_key, runtime_key);

        let manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_url.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );

        let node = manager
            .binding_nodes()
            .into_iter()
            .find(|candidate| candidate.key == binding_key)
            .expect("vless binding node should be present");

        assert!(node.alias_keys.contains(&runtime_key));
        assert!(node.alias_keys.contains(&legacy_alias));
    }

    #[test]
    fn binding_keys_ignore_transport_identity_changes_when_name_is_unique() {
        let proxy_a = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Falpha&sni=alpha.example.com#Tokyo%20Edge";
        let proxy_b = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fbeta&sni=beta.example.com#Tokyo%20Edge";
        assert_ne!(
            normalize_single_proxy_key(proxy_a),
            normalize_single_proxy_key(proxy_b),
            "runtime keys should still reflect transport identity"
        );

        let manager_a = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_a.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let manager_b = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_b.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );

        assert_eq!(
            current_binding_node(&manager_a).key,
            current_binding_node(&manager_b).key
        );
    }

    #[test]
    fn binding_keys_change_when_display_name_changes() {
        let proxy_a = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Falpha&sni=edge.example.com#Tokyo%20Edge";
        let proxy_b = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Falpha&sni=edge.example.com#Tokyo%20Edge%20Renamed";
        assert_eq!(
            normalize_single_proxy_key(proxy_a),
            normalize_single_proxy_key(proxy_b),
            "runtime keys should ignore display name changes"
        );

        let manager_a = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_a.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let manager_b = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_b.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );

        assert_ne!(
            current_binding_node(&manager_a).key,
            current_binding_node(&manager_b).key
        );
    }

    #[test]
    fn binding_keys_escalate_from_name_to_protocol_to_host_port() {
        let shadowsocks_url = |host| {
            format!(
                "{}{}{}{}{}",
                "ss://2022-blake3-aes-128-gcm:",
                "fixture-password",
                "@",
                host,
                ":8388#Shared%20Node"
            )
        };
        let protocol_split_manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![
                    "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#Shared%20Node".to_string(),
                    shadowsocks_url("ss.example.com"),
                ],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let protocol_nodes = protocol_split_manager
            .binding_nodes()
            .into_iter()
            .filter(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .collect::<Vec<_>>();
        assert_eq!(protocol_nodes.len(), 2);
        assert_ne!(protocol_nodes[0].key, protocol_nodes[1].key);

        let host_split_manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![
                    shadowsocks_url("jp-a.example.com"),
                    shadowsocks_url("jp-b.example.com"),
                ],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let host_nodes = host_split_manager
            .binding_nodes()
            .into_iter()
            .filter(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .collect::<Vec<_>>();
        assert_eq!(host_nodes.len(), 2);
        assert_ne!(host_nodes[0].key, host_nodes[1].key);

        let collapsed_manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![
                    "vless://11111111-1111-1111-1111-111111111111@shared.example.com:443?security=tls&type=ws&path=%2Falpha&sni=alpha.example.com#Shared%20Node".to_string(),
                    "vless://11111111-1111-1111-1111-111111111111@shared.example.com:443?security=tls&type=ws&path=%2Fbeta&sni=beta.example.com#Shared%20Node".to_string(),
                ],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let collapsed_nodes = collapsed_manager
            .binding_nodes()
            .into_iter()
            .filter(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .collect::<Vec<_>>();
        assert_eq!(collapsed_nodes.len(), 1);
        assert!(
            collapsed_nodes[0]
                .alias_keys
                .iter()
                .any(|key| key.starts_with("fpn_"))
        );
    }

    #[test]
    fn automatic_selection_does_not_use_direct() {
        let mut manager = ForwardProxyManager::new(ForwardProxySettings::default(), Vec::new());

        assert!(manager.select_auto_proxy().is_none());
    }

    #[test]
    fn pinned_selection_rejects_nodes_that_are_no_longer_bound_selectable() {
        let mut manager = manager_with_manual_proxy();
        let binding_key = current_binding_node(&manager).key;
        let endpoint_key = manager
            .bound_key_endpoint_keys
            .get(&binding_key)
            .cloned()
            .expect("binding key should resolve to an endpoint");
        let endpoint = manager
            .endpoints
            .iter_mut()
            .find(|endpoint| endpoint.key == endpoint_key)
            .expect("endpoint should exist");
        endpoint.endpoint_url = None;

        let err = manager
            .select_proxy_for_scope(&ForwardProxyRouteScope::pinned(binding_key))
            .expect_err("stale pinned node should be rejected");

        assert!(
            err.to_string()
                .contains("pinned forward proxy key is no longer available")
        );
    }

    #[test]
    fn current_bound_group_binding_key_normalizes_alias_backed_runtime_key() {
        let mut manager = manager_with_manual_proxy();
        let node = current_binding_node(&manager);
        let alias_key = node
            .alias_keys
            .first()
            .cloned()
            .expect("binding node should expose an alias key");
        manager.bound_group_runtime.insert(
            "latam".to_string(),
            BoundForwardProxyGroupState {
                current_binding_key: Some(alias_key),
                consecutive_network_failures: 0,
            },
        );

        assert_eq!(
            manager.current_bound_group_binding_key("latam", std::slice::from_ref(&node.key)),
            Some(node.key),
        );
    }

    #[test]
    fn bound_group_network_failures_can_switch_from_direct_to_proxy() {
        let mut manager = manager_with_manual_proxy();
        let binding_key = manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| node.key)
            .expect("missing non-direct binding node");
        let scope = ForwardProxyRouteScope::BoundGroup {
            group_name: "latam".to_string(),
            bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string(), binding_key.clone()],
        };
        manager.bound_group_runtime.insert(
            "latam".to_string(),
            BoundForwardProxyGroupState {
                current_binding_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                consecutive_network_failures: 0,
            },
        );

        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );
        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );
        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );

        let group_state = manager
            .bound_group_runtime
            .get("latam")
            .expect("missing bound group state after failures");
        assert_eq!(
            group_state.current_binding_key.as_deref(),
            Some(binding_key.as_str())
        );
        assert_eq!(group_state.consecutive_network_failures, 0);
    }

    #[tokio::test]
    async fn egress_ip_metadata_preserves_last_success_when_refresh_fails() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");
        let selected_proxy = SelectedForwardProxy {
            key: "jp-edge-01".to_string(),
            source: "manual".to_string(),
            display_name: "JP Edge 01".to_string(),
            endpoint_url: None,
            endpoint_url_raw: Some("http://jp-edge-01:8080".to_string()),
            egress_ip: None,
        };

        persist_forward_proxy_egress_ip_result(&pool, &selected_proxy, Some("203.0.113.24"), None)
            .await
            .expect("persist successful egress IP");
        persist_forward_proxy_egress_ip_result(
            &pool,
            &selected_proxy,
            None,
            Some("metadata refresh timed out"),
        )
        .await
        .expect("persist failed refresh");

        let metadata =
            load_forward_proxy_metadata_history(&pool, std::slice::from_ref(&selected_proxy.key))
                .await
                .expect("load metadata")
                .remove(&selected_proxy.key)
                .expect("metadata row");
        assert_eq!(metadata.egress_ip.as_deref(), Some("203.0.113.24"));
        assert_eq!(metadata.egress_ip_provider.as_deref(), Some("ipify"));
        assert_eq!(
            metadata.egress_ip_error.as_deref(),
            Some("metadata refresh timed out")
        );
        assert!(metadata.egress_ip_checked_at.is_some());
        assert!(metadata.egress_ip_error_at.is_some());
    }

    #[test]
    fn manual_latency_average_uses_only_successful_samples() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(101.4),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some("timeout".to_string()),
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some("timeout".to_string()),
            },
        );
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(198.6),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(250.0),
                ip: None,
                http_status: Some(200),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some("responses timeout".to_string()),
            },
        );

        assert_eq!(accumulator.completed_rounds, 2);
        assert_eq!(accumulator.success_count, 3);
        assert_eq!(accumulator.average_latency_ms(), Some(183));
    }

    #[test]
    fn manual_latency_average_is_none_when_all_rounds_fail() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        for _ in 0..forward_proxy_manual_latency_round_count() {
            accumulator.record_round(
                &ForwardProxyLatencyProbeTargetResult {
                    ok: false,
                    latency_ms: None,
                    ip: None,
                    http_status: None,
                    error: Some("egress timeout".to_string()),
                },
                &ForwardProxyLatencyProbeTargetResult {
                    ok: false,
                    latency_ms: None,
                    ip: None,
                    http_status: None,
                    error: Some("oauth timeout".to_string()),
                },
                &ForwardProxyLatencyProbeTargetResult {
                    ok: false,
                    latency_ms: None,
                    ip: None,
                    http_status: None,
                    error: Some("responses timeout".to_string()),
                },
            );
        }

        assert_eq!(accumulator.completed_rounds, 5);
        assert_eq!(accumulator.success_count, 0);
        assert_eq!(accumulator.average_latency_ms(), None);
    }

    #[test]
    fn manual_latency_requires_codex_responses_reachable() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        let codex_responses = ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some("responses timeout".to_string()),
        };
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(91.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(142.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &codex_responses,
        );

        assert_eq!(accumulator.success_count, 2);
        assert_eq!(accumulator.average_latency_ms(), Some(117));
        assert!(!accumulator.all_targets_ok());
        assert_eq!(
            accumulator.failed_targets(),
            vec![FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES]
        );
        assert_eq!(codex_responses.error.as_deref(), Some("responses timeout"));
    }

    #[test]
    fn manual_latency_codex_responses_accepts_method_not_allowed() {
        assert!(is_manual_latency_probe_reachable_status(
            StatusCode::METHOD_NOT_ALLOWED
        ));
        assert!(!is_manual_latency_probe_reachable_status(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
    }

    #[test]
    fn manual_latency_timeout_progress_preserves_completed_target_results() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(91.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(142.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(188.0),
                ip: None,
                http_status: Some(405),
                error: None,
            },
        );

        let progress =
            forward_proxy_latency_timeout_progress(&ForwardProxyEndpoint::direct(), &accumulator);

        assert!(progress.all_targets_ok);
        assert!(progress.failed_targets.is_empty());
        assert!(progress.egress_ip.ok);
        assert!(progress.oauth_upstream.ok);
        assert!(progress.codex_responses.ok);
        assert!(!progress.timed_out);
        assert_eq!(progress.average_latency_ms, Some(140));
    }

    #[test]
    fn manual_latency_preserves_failed_target_detail_after_later_success() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(91.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(142.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some("responses timeout".to_string()),
            },
        );
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(94.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(138.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(188.0),
                ip: None,
                http_status: Some(405),
                error: None,
            },
        );

        let progress =
            forward_proxy_latency_timeout_progress(&ForwardProxyEndpoint::direct(), &accumulator);

        assert!(!progress.all_targets_ok);
        assert_eq!(
            progress.failed_targets,
            vec![FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES]
        );
        assert!(!progress.codex_responses.ok);
        assert_eq!(
            progress.codex_responses.error.as_deref(),
            Some("responses timeout")
        );

        let current_success = ForwardProxyLatencyProbeTargetResult {
            ok: true,
            latency_ms: Some(188.0),
            ip: None,
            http_status: Some(405),
            error: None,
        };
        let (_, _, displayed_codex_responses) = accumulated_forward_proxy_latency_target_results(
            &accumulator,
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(94.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(138.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &current_success,
        );
        assert!(!displayed_codex_responses.ok);
        assert_eq!(
            displayed_codex_responses.error.as_deref(),
            Some("responses timeout")
        );
    }

    #[test]
    fn manual_latency_oauth_codex_probe_targets_preserve_base_path() {
        assert_eq!(
            oauth_codex_latency_probe_target("models")
                .expect("models target")
                .as_str(),
            "https://chatgpt.com/backend-api/codex/models"
        );
        assert_eq!(
            oauth_codex_latency_probe_target("responses")
                .expect("responses target")
                .as_str(),
            "https://chatgpt.com/backend-api/codex/responses"
        );
    }

    #[test]
    fn manual_latency_batch_schedule_is_breadth_first() {
        assert_eq!(
            forward_proxy_latency_breadth_first_schedule(3, 2),
            vec![(1, 0), (1, 1), (1, 2), (2, 0), (2, 1), (2, 2)]
        );
    }

    #[test]
    fn manual_latency_batch_query_accepts_repeated_keys() {
        assert_eq!(
            parse_forward_proxy_nodes_latency_test_keys(
                "key=__direct__&key=fpn_a&ignored=value&key=fpn_b"
            ),
            vec![
                "__direct__".to_string(),
                "fpn_a".to_string(),
                "fpn_b".to_string()
            ]
        );
    }
}
