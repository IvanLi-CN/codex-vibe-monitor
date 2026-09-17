#[cfg(test)]
mod dashboard_activity_routing_tests {
    use super::*;

    fn test_materialized_base() -> DashboardActivityTopicMaterializedBase {
        let snapshot = DashboardActivitySnapshot::test_stub("today");
        let range = ExactUtcRange {
            start: snapshot.range_start,
            end: snapshot.range_end,
        };
        let delta = dashboard_activity_terminal_delta(&summary_projection_test_invocation());
        let mut account = dashboard_activity_terminal_account_for_range(range, &delta);
        account.effective_routing_rule = Some(default_effective_routing_rule());
        account.recent_invocations = vec![PromptCacheConversationInvocationPreviewResponse {
            id: 7,
            invoke_id: "routing-rule-preview".to_string(),
            prompt_cache_key: Some("routing-rule-cache".to_string()),
            occurred_at: "2026-07-20 10:25:09".to_string(),
            status: "success".to_string(),
            live_phase: None,
            failure_class: None,
            route_mode: Some("pool".to_string()),
            model: Some("gpt-5.4".to_string()),
            request_model: Some("gpt-5.4".to_string()),
            response_model: Some("gpt-5.4".to_string()),
            total_tokens: 1_200,
            cost: Some(0.0099),
            proxy_display_name: Some("ciii".to_string()),
            upstream_account_id: Some(17),
            upstream_account_name: Some("Pool 17".to_string()),
            upstream_account_plan_type: None,
            endpoint: Some("/v1/responses".to_string()),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
            source: Some(SOURCE_PROXY.to_string()),
            input_tokens: Some(1_000),
            output_tokens: Some(200),
            cache_input_tokens: Some(400),
            reasoning_tokens: None,
            reasoning_effort: Some("medium".to_string()),
            error_message: None,
            downstream_status_code: Some(200),
            downstream_error_message: None,
            failure_kind: None,
            blocked_binding: None,
            is_actionable: Some(false),
            response_content_encoding: Some("identity".to_string()),
            request_compression_algorithm: Some("zstd".to_string()),
            transport: Some("http".to_string()),
            requested_service_tier: Some("default".to_string()),
            service_tier: Some("default".to_string()),
            billing_service_tier: Some("default".to_string()),
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: Some(720.0),
            t_upstream_stream_ms: Some(500.0),
            t_resp_parse_ms: None,
            t_persist_ms: None,
            t_total_ms: Some(1_200.0),
        }];
        let version = RoutingStateVersion {
            epoch: "2026-09-05T12:00:00.000000000Z".to_string(),
            generation: "4".to_string(),
        };
        DashboardActivityTopicMaterializedBase {
            response: DashboardActivityResponse {
                range: snapshot.range,
                range_start: format_utc_iso_precise(snapshot.range_start),
                range_end: format_utc_iso_precise(snapshot.range_end),
                snapshot_id: snapshot.range_end.timestamp_millis(),
                routing_state_version: Some(version),
                terminal_sequence: snapshot.terminal_sequence,
                live_revision: 3,
                rate_window: DashboardActivityRateWindowResponse {
                    start: format_utc_iso_precise(snapshot.range_start),
                    end: format_utc_iso_precise(snapshot.range_end),
                    window_minutes: 1,
                    mode: "rolling_60s_live_mean".to_string(),
                },
                summary: snapshot.summary,
                network_live_bucket: None,
                network_realtime_rate: None,
                accounts: Some(vec![account]),
            },
            summary_model_performance_accumulator: snapshot.summary_model_performance_accumulator,
            account_model_performance_accumulators: snapshot.account_model_performance_accumulators,
            account_latency_accumulators: snapshot.account_latency_accumulators,
            model_performance_accumulator_ready: snapshot.model_performance_accumulator_ready,
            recent_limit: 16,
            include_recent: true,
        }
    }

    #[test]
    fn routing_rule_live_projection_updates_only_rule_and_version() {
        let mut base = test_materialized_base();
        let before_summary = base.response.summary.stats.total_count;
        let before_recent = base
            .response
            .accounts
            .as_ref()
            .expect("account activity")
            .first()
            .expect("account")
            .recent_invocations
            .len();
        let mut primary = default_effective_routing_rule();
        primary.priority_tier = TagPriorityTier::Primary;
        let version = RoutingStateVersion {
            epoch: "2026-09-05T12:00:00.000000000Z".to_string(),
            generation: "5".to_string(),
        };

        assert!(base.apply_routing_rule_change(&version, &[(17, primary)], &[]));
        {
            let account = base
                .response
                .accounts
                .as_ref()
                .expect("account activity")
                .first()
                .expect("account");
            assert_eq!(
                account
                    .effective_routing_rule
                    .as_ref()
                    .unwrap()
                    .priority_tier,
                TagPriorityTier::Primary
            );
            assert_eq!(account.recent_invocations.len(), before_recent);
        }
        assert_eq!(
            base.response
                .routing_state_version
                .as_ref()
                .unwrap()
                .generation,
            "5"
        );
        assert_eq!(base.response.summary.stats.total_count, before_summary);

        let mut stale = default_effective_routing_rule();
        stale.priority_tier = TagPriorityTier::Fallback;
        assert!(!base.apply_routing_rule_change(&version, &[(17, stale)], &[]));
        assert_eq!(
            base.response
                .accounts
                .as_ref()
                .expect("account activity")
                .first()
                .expect("account")
                .effective_routing_rule
                .as_ref()
                .unwrap()
                .priority_tier,
            TagPriorityTier::Primary
        );
    }
}
