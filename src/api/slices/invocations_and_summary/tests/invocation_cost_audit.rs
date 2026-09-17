#[cfg(test)]
mod invocation_cost_audit_tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_pricing_catalog() -> PricingCatalog {
        PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4".to_string(),
                ModelPricing {
                    input_per_1m: 1.0,
                    output_per_1m: 2.0,
                    cache_input_per_1m: Some(0.1),
                    cache_read_per_1m: Some(0.2),
                    cache_write_per_1m: Some(1.25),
                    reasoning_per_1m: Some(3.0),
                    source: "unit-test".to_string(),
                },
            )]),
        }
    }

    pub(super) fn sample_invocation(reasoning_tokens: Option<i64>) -> ApiInvocation {
        ApiInvocation {
            id: 7,
            invoke_id: "invocation-cost-audit".to_string(),
            occurred_at: "2026-07-20 10:25:09".to_string(),
            source: SOURCE_PROXY.to_string(),
            proxy_display_name: Some("ciii".to_string()),
            model: Some("gpt-5.4".to_string()),
            request_model: Some("gpt-5.4".to_string()),
            response_model: Some("gpt-5.4".to_string()),
            input_tokens: Some(1_000),
            output_tokens: Some(200),
            cache_input_tokens: Some(400),
            reasoning_tokens,
            reasoning_effort: Some("medium".to_string()),
            total_tokens: Some(1_200),
            cost: Some(0.0099),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: None,
            status: Some("success".to_string()),
            live_phase: None,
            error_message: None,
            downstream_status_code: Some(200),
            failure_kind: None,
            blocked_binding: None,
            blocked_binding_json: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: Some("req_cost_audit".to_string()),
            failure_class: Some("none".to_string()),
            is_actionable: Some(false),
            endpoint: Some("/v1/responses".to_string()),
            compaction_request_kind: Some("remote_v2".to_string()),
            compaction_response_kind: Some("remote_v2".to_string()),
            image_intent: Some("no".to_string()),
            requester_ip: Some("192.168.31.6".to_string()),
            prompt_cache_key: Some("pck-cost-audit".to_string()),
            sticky_key: None,
            route_mode: Some("pool".to_string()),
            upstream_account_id: Some(17),
            upstream_account_name: Some("Pool 17".to_string()),
            response_content_encoding: Some("identity".to_string()),
            request_compression_algorithm: Some("zstd".to_string()),
            transport: Some("http".to_string()),
            pool_attempt_count: Some(3),
            pool_distinct_account_count: Some(2),
            pool_attempt_terminal_reason: Some("success".to_string()),
            requested_service_tier: Some("default".to_string()),
            service_tier: Some("default".to_string()),
            billing_service_tier: Some("default".to_string()),
            proxy_weight_delta: None,
            cost_estimated: Some(0),
            price_version: Some("legacy@response-tier".to_string()),
            cost_audit: None,
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            detail_level: "full".to_string(),
            detail_pruned_at: None,
            detail_prune_reason: None,
            t_total_ms: Some(3_450.0),
            t_req_read_ms: Some(20.0),
            t_req_parse_ms: Some(8.0),
            t_upstream_connect_ms: Some(190.0),
            t_upstream_ttfb_ms: Some(330.0),
            first_token_ms: None,
            t_upstream_stream_ms: Some(2_800.0),
            t_resp_parse_ms: Some(12.0),
            t_persist_ms: Some(6.0),
            created_at: "2026-07-20 10:25:09".to_string(),
        }
    }

    fn sample_attempt_row(attempt_index: i64, status: &str) -> InvocationWorkflowAttemptRow {
        InvocationWorkflowAttemptRow {
            attempt_row_id: attempt_index,
            attempt_id: Some(format!("attempt-{attempt_index}")),
            invoke_id: "invocation-cost-audit".to_string(),
            occurred_at: "2026-07-20 10:25:09".to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: None,
            routing_source: None,
            routing_selection_audit_json: None,
            upstream_account_id: Some(17),
            upstream_account_name: Some("Pool 17".to_string()),
            upstream_route_key: Some("route-17".to_string()),
            proxy_binding_key_snapshot: Some("binding-17".to_string()),
            attempt_index,
            distinct_account_index: attempt_index,
            same_account_retry_index: 0,
            requester_ip: Some("192.168.31.6".to_string()),
            started_at: Some("2026-07-20 10:25:09".to_string()),
            finished_at: Some("2026-07-20 10:25:12".to_string()),
            status: status.to_string(),
            phase: Some(if status == "failed" {
                "streaming".to_string()
            } else {
                "completed".to_string()
            }),
            http_status: Some(if status == "failed" { 500 } else { 200 }),
            downstream_http_status: Some(200),
            failure_kind: (status == "failed").then_some("upstream_error".to_string()),
            error_message: (status == "failed").then_some("upstream error".to_string()),
            downstream_error_message: None,
            connect_latency_ms: Some(120.0),
            first_byte_latency_ms: Some(240.0),
            stream_latency_ms: Some(1_500.0),
            upstream_request_id: Some(format!("req-{attempt_index}")),
            upstream_request_compression_algorithm: None,
            upstream_request_compression_mode: None,
            upstream_request_logical_body_bytes: None,
            upstream_request_transmitted_body_bytes: None,
            upstream_request_header_bytes_approx: None,
            upstream_response_body_bytes: None,
            upstream_response_header_bytes_approx: None,
            compact_support_status: None,
            compact_support_reason: None,
            request_summary_json: None,
            response_summary_json: None,
            response_raw_path: None,
            response_raw_codec: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            response_content_encoding: None,
        }
    }

    fn assert_close(left: f64, right: f64) {
        assert!(
            (left - right).abs() < 1e-12,
            "expected {left} to be close to {right}"
        );
    }

    #[test]
    fn build_invocation_cost_audit_flags_price_version_change_and_keeps_total_only_history() {
        let record = sample_invocation(None);
        let catalog = sample_pricing_catalog();

        let audit =
            build_invocation_cost_audit(&record, &catalog, true).expect("cost audit should exist");

        assert!(audit.mismatch);
        assert_eq!(
            audit.reason.as_deref(),
            Some(INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED)
        );
        assert_eq!(
            audit.recorded_price_version.as_deref(),
            Some("legacy@response-tier")
        );
        assert_eq!(
            audit.local_price_version.as_deref(),
            Some("unit-test@response-tier")
        );

        let recorded = audit.recorded.expect("recorded breakdown");
        assert_eq!(recorded.total, Some(0.0099));
        assert_eq!(recorded.input, None);
        assert_eq!(recorded.cache_write, None);
        assert_eq!(recorded.cache_read, None);
        assert_eq!(recorded.output, None);
        assert_eq!(recorded.reasoning, None);

        let local = audit.local.expect("local breakdown");
        assert_close(local.input.expect("input cost"), 0.0);
        assert_close(local.cache_write.expect("cache write cost"), 0.00075);
        assert_close(local.cache_read.expect("cache read cost"), 0.00008);
        assert_close(local.output.expect("output cost"), 0.0004);
        assert_close(local.reasoning.expect("reasoning cost"), 0.0);
        assert_close(local.total.expect("total cost"), 0.00123);
        assert!(
            audit.absolute_diff_usd.expect("absolute diff")
                > INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD
        );
    }

    #[test]
    fn build_invocation_usage_summary_preserves_reasoning_null_vs_zero() {
        let catalog = sample_pricing_catalog();

        let record_without_reasoning = sample_invocation(None);
        let audit_without_reasoning =
            build_invocation_cost_audit(&record_without_reasoning, &catalog, true)
                .expect("cost audit for null reasoning");
        let usage_without_reasoning =
            build_invocation_usage_summary(&record_without_reasoning, &audit_without_reasoning);
        assert!(usage_without_reasoning["reasoningTokens"].is_null());
        assert!(usage_without_reasoning["tokens"]["reasoning"].is_null());

        let record_with_zero_reasoning = sample_invocation(Some(0));
        let audit_with_zero_reasoning =
            build_invocation_cost_audit(&record_with_zero_reasoning, &catalog, true)
                .expect("cost audit for zero reasoning");
        let usage_with_zero_reasoning =
            build_invocation_usage_summary(&record_with_zero_reasoning, &audit_with_zero_reasoning);
        assert_eq!(
            usage_with_zero_reasoning["reasoningTokens"].as_i64(),
            Some(0)
        );
        assert_eq!(
            usage_with_zero_reasoning["tokens"]["reasoning"].as_i64(),
            Some(0)
        );
    }

    #[test]
    fn workflow_usage_audit_only_attaches_to_last_success_like_attempt() {
        let record = sample_invocation(Some(0));
        let catalog = sample_pricing_catalog();
        let usage_cost_audit =
            build_invocation_cost_audit(&record, &catalog, true).expect("cost audit");

        let attempt_rows = [
            sample_attempt_row(1, "failed"),
            sample_attempt_row(2, "success"),
            sample_attempt_row(3, "warning_success"),
        ];
        let attempt_refs = attempt_rows.iter().collect::<Vec<_>>();
        let last_success_attempt_row_id = last_success_like_attempt_row_id(&attempt_refs);
        assert_eq!(last_success_attempt_row_id, Some(3));

        let attempts = attempt_rows
            .iter()
            .map(|attempt| {
                build_workflow_attempt_from_row(
                    &record,
                    attempt,
                    None,
                    (last_success_attempt_row_id == Some(attempt.attempt_row_id))
                        .then_some(&usage_cost_audit),
                    attempt.attempt_index == 3,
                )
            })
            .collect::<Vec<_>>();

        for attempt in &attempts[..2] {
            let response_summary = attempt.response_summary.as_ref().expect("response summary");
            assert!(
                response_summary.get("usage").is_some_and(Value::is_null),
                "non-terminal success attempts should not receive usage audit"
            );
        }

        let final_response_summary = attempts[2]
            .response_summary
            .as_ref()
            .expect("final response summary");
        assert_eq!(
            final_response_summary["usage"]["reasoningTokens"].as_i64(),
            Some(0)
        );
        assert_eq!(
            final_response_summary["usage"]["audit"]["reason"].as_str(),
            Some(INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED)
        );
    }

    #[test]
    fn final_real_attempt_uses_row_id_to_break_shared_attempt_index_ties() {
        let mut record = sample_invocation(Some(0));
        record.first_token_ms = Some(840.0);
        let mut earlier_retry = sample_attempt_row(1, "failed");
        earlier_retry.attempt_row_id = 41;
        earlier_retry.same_account_retry_index = 0;
        let mut final_retry = sample_attempt_row(1, "success");
        final_retry.attempt_row_id = 42;
        final_retry.same_account_retry_index = 1;
        let attempts = [&earlier_retry, &final_retry];

        let final_attempt_row_id = final_real_attempt_row_id(&attempts);
        assert_eq!(final_attempt_row_id, Some(42));

        let mapped = attempts
            .iter()
            .map(|attempt| {
                build_workflow_attempt_from_row(
                    &record,
                    attempt,
                    None,
                    None,
                    final_attempt_row_id == Some(attempt.attempt_row_id),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(mapped[0].first_token_ms, None);
        assert_eq!(mapped[1].first_token_ms, Some(840.0));
    }

    #[test]
    fn final_real_attempt_excludes_budget_exhausted_marker() {
        let mut successful_attempt = sample_attempt_row(1, "success");
        successful_attempt.attempt_row_id = 41;
        let mut budget_marker = sample_attempt_row(2, "budget_exhausted_final");
        budget_marker.attempt_row_id = 42;
        budget_marker.request_summary_json = Some("{\"retry\":true}".to_string());

        assert_eq!(
            final_real_attempt_row_id(&[&successful_attempt, &budget_marker]),
            Some(41)
        );
    }

    #[test]
    fn terminal_retry_projection_keeps_final_first_token_measurement() {
        let mut record = sample_invocation(None);
        record.id = 0;
        record.status = Some("success".to_string());
        record.first_token_ms = Some(720.0);
        record.pool_attempt_count = Some(2);

        assert_eq!(runtime_record_first_token_ms(&record), Some(720.0));
        assert_eq!(
            dashboard_activity_terminal_delta(&record).first_token_ms,
            Some(720.0)
        );
    }

    #[test]
    fn failed_terminal_retry_projection_hides_unowned_first_token_measurement() {
        let mut record = sample_invocation(None);
        record.status = Some("failed".to_string());
        record.first_token_ms = Some(720.0);
        record.t_upstream_stream_ms = None;
        record.pool_attempt_count = Some(2);

        assert_eq!(runtime_record_first_token_ms(&record), None);
        assert_eq!(
            dashboard_activity_terminal_delta(&record).first_token_ms,
            None
        );
    }

    #[test]
    fn streamed_failed_terminal_retry_keeps_final_first_token_measurement() {
        let mut record = sample_invocation(None);
        record.id = 0;
        record.status = Some("failed".to_string());
        record.first_token_ms = Some(720.0);
        record.t_upstream_stream_ms = Some(240.0);
        record.pool_attempt_count = Some(2);

        assert_eq!(runtime_record_first_token_ms(&record), Some(720.0));
        assert_eq!(
            dashboard_activity_terminal_delta(&record).first_token_ms,
            Some(720.0)
        );
    }

    #[test]
    fn http_200_error_retry_hides_unowned_first_token_measurement() {
        let mut record = sample_invocation(None);
        record.id = 0;
        record.status = Some("http_200".to_string());
        record.error_message = Some("upstream parse failed".to_string());
        record.first_token_ms = Some(720.0);
        record.t_upstream_stream_ms = None;
        record.pool_attempt_count = Some(2);

        assert_eq!(runtime_record_first_token_ms(&record), None);
        assert_eq!(
            dashboard_activity_terminal_delta(&record).first_token_ms,
            None
        );
    }

    #[test]
    fn persisted_retry_stream_timing_cannot_authorize_runtime_ttft() {
        let mut record = sample_invocation(None);
        record.status = Some("failed".to_string());
        record.first_token_ms = Some(720.0);
        record.t_upstream_stream_ms = Some(240.0);
        record.pool_attempt_count = Some(2);

        assert_eq!(runtime_record_first_token_ms(&record), None);
        assert_eq!(
            dashboard_activity_terminal_delta(&record).first_token_ms,
            None
        );
    }

    #[test]
    fn persisted_dashboard_delta_keeps_prequalified_final_retry_ttft() {
        let mut record = sample_invocation(None);
        record.status = Some("success".to_string());
        record.first_token_ms = Some(720.0);
        record.t_upstream_stream_ms = Some(240.0);
        record.pool_attempt_count = Some(2);

        assert_eq!(
            persisted_dashboard_activity_terminal_delta(&record).first_token_ms,
            Some(720.0)
        );
    }

    #[test]
    fn final_attempt_status_without_stream_evidence_hides_invocation_timings() {
        let mut record = sample_invocation(Some(0));
        record.first_token_ms = Some(840.0);
        let mut final_attempt = sample_attempt_row(1, "success");
        final_attempt.first_byte_latency_ms = None;
        final_attempt.stream_latency_ms = None;

        let mapped = build_workflow_attempt_from_row(&record, &final_attempt, None, None, true);

        assert_eq!(mapped.first_token_ms, None);
        assert_eq!(mapped.stream_latency_ms, None);

        let summary = build_attempt_response_summary(&record, &final_attempt, None, None, true);
        assert!(summary["latencyMs"]["stream"].is_null());
    }

    #[test]
    fn pending_streaming_final_attempt_keeps_measured_ttft_without_stream_duration() {
        let mut record = sample_invocation(None);
        record.status = Some("running".to_string());
        record.first_token_ms = Some(720.0);
        let mut final_attempt = sample_attempt_row(1, "pending");
        final_attempt.phase = Some("streaming_response".to_string());
        final_attempt.finished_at = None;
        final_attempt.first_byte_latency_ms = Some(180.0);
        final_attempt.stream_latency_ms = None;

        let mapped = build_workflow_attempt_from_row(&record, &final_attempt, None, None, true);

        assert_eq!(mapped.first_token_ms, Some(720.0));
        assert_eq!(mapped.stream_latency_ms, None);
    }

    #[test]
    fn pending_streaming_retry_keeps_measured_final_attempt_ttft() {
        let mut record = sample_invocation(None);
        record.status = Some("running".to_string());
        record.first_token_ms = Some(720.0);
        let mut final_attempt = sample_attempt_row(2, "pending");
        final_attempt.phase = Some("streaming_response".to_string());
        final_attempt.finished_at = None;
        final_attempt.first_byte_latency_ms = Some(180.0);
        final_attempt.stream_latency_ms = None;

        let mapped = build_workflow_attempt_from_row(&record, &final_attempt, None, None, true);

        assert_eq!(mapped.first_token_ms, Some(720.0));
        assert_eq!(mapped.stream_latency_ms, None);
    }

    #[test]
    fn runtime_account_preview_suppresses_invocation_ttft_after_retry() {
        let mut record = sample_invocation(None);
        record.status = Some("running".to_string());
        record.first_token_ms = Some(720.0);
        record.pool_attempt_count = Some(2);

        let row = runtime_upstream_account_activity_preview_row(record, InvocationSourceScope::All)
            .expect("running pool invocation preview");

        assert_eq!(row.first_token_ms, None);
        assert_ne!(
            row.live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn retry_runtime_projection_does_not_infer_responding_from_invocation_ttft() {
        let mut record = sample_invocation(None);
        record.status = Some("running".to_string());
        record.first_token_ms = Some(720.0);
        record.pool_attempt_count = Some(2);

        assert_eq!(runtime_record_first_token_ms(&record), None);
        assert_ne!(
            runtime_record_live_phase(&record),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn current_runtime_retry_can_report_measured_ttft() {
        let mut record = sample_invocation(None);
        record.id = 0;
        record.status = Some("running".to_string());
        record.first_token_ms = Some(720.0);
        record.pool_attempt_count = Some(2);

        assert_eq!(runtime_record_first_token_ms(&record), Some(720.0));
        assert_eq!(
            runtime_record_live_phase(&record),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn retry_runtime_projection_can_use_baseline_retry_state() {
        let mut record = sample_invocation(None);
        record.status = Some("running".to_string());
        record.first_token_ms = Some(720.0);

        assert_eq!(
            runtime_record_first_token_ms_with_retry(&record, true),
            None
        );
        assert_ne!(
            runtime_record_live_phase_with_retry(&record, true),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn runtime_account_preview_keeps_invocation_ttft_for_first_attempt() {
        let mut record = sample_invocation(None);
        record.status = Some("running".to_string());
        record.first_token_ms = Some(720.0);
        record.pool_attempt_count = Some(1);

        let row = runtime_upstream_account_activity_preview_row(record, InvocationSourceScope::All)
            .expect("running pool invocation preview");

        assert_eq!(row.first_token_ms, Some(720.0));
        assert_eq!(
            row.live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn stored_response_summary_cannot_restore_missing_final_stream_evidence() {
        let record = sample_invocation(None);
        let mut final_attempt = sample_attempt_row(1, "success");
        final_attempt.stream_latency_ms = None;
        final_attempt.response_summary_json = Some(r#"{"latencyMs":{"stream":42.0}}"#.to_string());

        let mapped = build_workflow_attempt_from_row(&record, &final_attempt, None, None, true);

        assert!(
            mapped.response_summary.expect("stored response summary")["latencyMs"]["stream"]
                .is_null()
        );
    }

    #[test]
    fn pending_runtime_phase_ignores_stale_requesting_metadata() {
        let mut record = sample_invocation(None);
        record.status = Some("pending".to_string());
        record.live_phase = Some(INVOCATION_LIVE_PHASE_REQUESTING.to_string());

        assert_eq!(
            effective_runtime_invocation_live_phase(&record),
            Some(INVOCATION_LIVE_PHASE_QUEUED)
        );
    }

    #[test]
    fn attempt_response_summary_uses_its_own_raw_capture_metadata() {
        let mut record = sample_invocation(Some(0));
        record.response_raw_path = Some("response-body.json".to_string());
        record.response_raw_size = Some(181_382);
        record.response_content_encoding = Some("identity".to_string());
        record.upstream_request_id = Some("W1scc2SS".to_string());

        let mut attempt = sample_attempt_row(1, "failed");
        attempt.upstream_request_id = Some("H3HxTB12".to_string());
        attempt.upstream_response_body_bytes = Some(98);
        attempt.response_raw_path = Some("attempt-H3HxTB12-response.bin".to_string());
        attempt.response_raw_size = Some(98);
        attempt.response_raw_truncated = Some(0);
        attempt.response_content_encoding = Some("gzip".to_string());

        let response_summary = build_attempt_response_summary(&record, &attempt, None, None, false);

        assert_eq!(
            response_summary["responseBodyCapture"]["availableAtInvocationLevel"],
            Value::Bool(false)
        );
        assert_eq!(
            response_summary["responseBodyCapture"]["availableAtAttemptLevel"],
            Value::Bool(true)
        );
        assert_eq!(response_summary["responseBodyCapture"]["size"], json!(98));
        assert_eq!(
            response_summary["responseBodyCapture"]["detailLevel"],
            json!("full")
        );
        assert!(response_summary["responseBodyCapture"]["unavailableReason"].is_null());
        assert_eq!(
            response_summary["headers"]["upstreamRequestId"],
            json!("H3HxTB12")
        );
        assert_eq!(
            response_summary["headers"]["contentEncoding"],
            json!("gzip")
        );
        assert_eq!(response_summary["responseContentEncoding"], json!("gzip"));
    }

    #[test]
    fn workflow_attempt_mapping_scopes_invocation_response_capture_to_final_attempt() {
        let mut record = sample_invocation(Some(0));
        record.response_raw_path = Some("response-body.json".to_string());
        record.response_raw_size = Some(181_382);
        record.response_raw_truncated = Some(1);
        record.response_content_encoding = Some("identity".to_string());
        record.upstream_request_id = Some("W1scc2SS".to_string());

        let mut first_attempt = sample_attempt_row(1, "failed");
        first_attempt.upstream_request_id = Some("H3HxTB12".to_string());
        first_attempt.upstream_response_body_bytes = Some(98);
        let mut final_attempt = sample_attempt_row(2, "success");
        final_attempt.upstream_request_id = Some("W1scc2SS".to_string());
        final_attempt.upstream_response_body_bytes = Some(181_382);

        let first = build_workflow_attempt_from_row(&record, &first_attempt, None, None, false);
        let final_attempt =
            build_workflow_attempt_from_row(&record, &final_attempt, None, None, true);

        assert_eq!(
            first.response_summary.as_ref().expect("first summary")["headers"]["upstreamRequestId"],
            json!("H3HxTB12")
        );
        assert_eq!(
            first.response_summary.as_ref().expect("first summary")["responseBodyCapture"]["size"],
            json!(98)
        );
        assert_eq!(
            final_attempt
                .response_summary
                .as_ref()
                .expect("final summary")["headers"]["upstreamRequestId"],
            json!("W1scc2SS")
        );
        assert_eq!(
            final_attempt
                .response_summary
                .as_ref()
                .expect("final summary")["responseBodyCapture"]["size"],
            json!(181_382)
        );
        assert_eq!(
            final_attempt
                .response_summary
                .as_ref()
                .expect("final summary")["responseBodyCapture"]["availableAtInvocationLevel"],
            Value::Bool(true)
        );
        assert_eq!(
            final_attempt
                .response_summary
                .as_ref()
                .expect("final summary")["responseBodyCapture"]["detailLevel"],
            json!("full")
        );
        assert_eq!(
            final_attempt
                .response_summary
                .as_ref()
                .expect("final summary")["responseBodyCapture"]["truncated"],
            json!(true)
        );
    }

    #[test]
    fn workflow_attempt_request_summary_keeps_its_own_codex_imagegen_audit() {
        let record = sample_invocation(Some(0));
        let mut first_attempt = sample_attempt_row(1, "failed");
        first_attempt.request_summary_json = Some(
            json!({
                "codexImagegenRewrite": {"protocol": "full", "mode": "force_add"}
            })
            .to_string(),
        );
        let mut final_attempt = sample_attempt_row(2, "success");
        final_attempt.request_summary_json = Some(
            json!({
                "codexImagegenRewrite": {"protocol": "lite", "mode": "force_remove"}
            })
            .to_string(),
        );
        let invocation_payload = json!({
            "codexImagegenRewrite": {"protocol": "lite", "mode": "force_add"}
        });

        let first = build_workflow_attempt_from_row(
            &record,
            &first_attempt,
            Some(&invocation_payload),
            None,
            false,
        );
        let final_attempt = build_workflow_attempt_from_row(
            &record,
            &final_attempt,
            Some(&invocation_payload),
            None,
            true,
        );

        assert_eq!(
            first
                .request_summary
                .as_ref()
                .expect("first request summary")["codexImagegenRewrite"]["protocol"],
            json!("full")
        );
        assert_eq!(
            final_attempt
                .request_summary
                .as_ref()
                .expect("final request summary")["codexImagegenRewrite"]["mode"],
            json!("force_remove")
        );
        assert_eq!(
            first
                .request_summary
                .as_ref()
                .expect("first request summary")["endpoint"],
            json!("/v1/responses")
        );
    }
}
