async fn reserve_sticky_model_route(
    state: &AppState,
    reservation_key: Option<&str>,
    account: &PoolResolvedAccount,
    requested_model: Option<&str>,
) -> Result<bool> {
    let Some(reservation_key) = reservation_key else {
        return Ok(true);
    };
    let concurrency_limit =
        model_route_concurrency_limit(&state.pool, account.account_id, requested_model).await?;
    Ok(try_reserve_pool_routing_account_for_model(
        state,
        reservation_key,
        account,
        requested_model,
        concurrency_limit,
    ))
}

pub(crate) fn request_capability_requirements_after_codex_imagegen_rewrite(
    endpoint: &str,
    image_intent: crate::ImageIntent,
    requested_model: Option<&str>,
    codex_imagegen_request: bool,
    rule: &EffectiveRoutingRule,
) -> RequestCapabilityRequirements {
    let hosted_image_intent = if codex_imagegen_request
        && rule.codex_imagegen_rewrite_mode != crate::CodexImagegenRewriteMode::KeepOriginal
        && image_intent == crate::ImageIntent::Yes
        && !requested_model.is_some_and(crate::is_openai_image_generation_model)
    {
        crate::ImageIntent::No
    } else {
        image_intent
    };
    let mut requirements = RequestCapabilityRequirements::from_endpoint_and_image_intent(
        endpoint,
        hosted_image_intent,
    );
    let codex_imagegen_rewrite_applies = match rule.codex_imagegen_rewrite_mode {
        crate::CodexImagegenRewriteMode::ForceAdd => true,
        crate::CodexImagegenRewriteMode::FillMissing => image_intent == crate::ImageIntent::Yes,
        crate::CodexImagegenRewriteMode::KeepOriginal
        | crate::CodexImagegenRewriteMode::ForceRemove => false,
    };
    requirements.codex_imagegen = codex_imagegen_request
        && codex_imagegen_rewrite_applies
        && matches!(endpoint, "/v1/responses" | "/v1/responses/compact");
    requirements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routing_score(account_id: i64) -> PoolRoutingCandidateScore {
        PoolRoutingCandidateScore {
            eligibility: PoolRoutingCandidateEligibility::Assignable,
            route_binding_failure_penalty: 0,
            model_route_penalty: 0,
            routing_priority_rank: 0,
            capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
            dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
            single_account_rotation_enabled: false,
            secondary_reset_proximity_secs: None,
            primary_reset_proximity_secs: None,
            scarcity_score: 0.0,
            effective_load: 0,
            last_selected_at: None,
            account_id,
        }
    }

    #[test]
    fn routing_selection_audit_winner_reason_matches_retry_original_precedence() {
        let mut winner = routing_score(1);
        winner.capacity_lane = PoolRoutingCandidateCapacityLane::Overflow;
        let mut runner_up = routing_score(2);
        runner_up.dispatch_state = PoolRoutingCandidateDispatchState::RetryOriginalNode;

        assert!(compare_pool_routing_candidate_scores(&winner, &runner_up).is_lt());
        assert_eq!(
            pool_routing_selection_winner_reason(&winner, Some(&runner_up)),
            "avoidsRetryOriginalNode"
        );
    }

    #[test]
    fn routing_selection_audit_winner_reason_ignores_reset_proximity_without_rotation() {
        let mut winner = routing_score(1);
        winner.secondary_reset_proximity_secs = Some(30);
        let mut runner_up = routing_score(2);
        runner_up.secondary_reset_proximity_secs = Some(10);

        assert!(compare_pool_routing_candidate_scores(&winner, &runner_up).is_lt());
        assert_eq!(
            pool_routing_selection_winner_reason(&winner, Some(&runner_up)),
            "stableAccountOrder"
        );
    }

    #[test]
    fn routing_selection_score_snapshot_preserves_the_compared_penalties() {
        let mut winner = routing_score(1);
        winner.model_route_penalty = 0;
        let mut runner_up = routing_score(2);
        runner_up.model_route_penalty = 1;

        let audit = PoolRoutingSelectionAudit {
            selected_account_id: winner.account_id,
            selected_account_name: "dzw".to_string(),
            eligible_candidate_count: 2,
            winner_reason_code: "lowerModelRoutePenalty".to_string(),
            compared_account_id: Some(runner_up.account_id),
            compared_account_name: Some("CIII".to_string()),
            selected_score: Some(routing_selection_score_snapshot(&winner)),
            compared_score: Some(routing_selection_score_snapshot(&runner_up)),
            handoff_admission: None,
            excluded_candidates: Vec::new(),
        };
        let value = serde_json::to_value(audit).expect("serialize routing selection audit");
        assert_eq!(value["selectedScore"]["modelRoutePenalty"], 0);
        assert_eq!(value["selectedScore"]["modelRoutePenaltyCode"], "normal");
        assert_eq!(value["comparedScore"]["modelRoutePenalty"], 1);
        assert_eq!(value["comparedScore"]["modelRoutePenaltyCode"], "demoted");
    }

    #[test]
    fn no_candidate_audit_keeps_full_reason_counts_with_bounded_details() {
        let exclusions = (1..=15)
            .map(|account_id| PoolRoutingSelectionAuditExcludedCandidate {
                account_id,
                account_name: format!("Account {account_id}"),
                reason_code: "policyExcluded".to_string(),
            })
            .collect::<Vec<_>>();

        let audit = no_candidate_audit("policyExcluded", 15, 0, 0, &exclusions);

        assert_eq!(audit.excluded_reason_counts["policyExcluded"], 15);
        assert_eq!(
            audit.candidates.len(),
            POOL_ROUTING_SELECTION_AUDIT_EXCLUSION_LIMIT
        );
    }

    fn effective_rule(
        codex_imagegen_rewrite_mode: crate::CodexImagegenRewriteMode,
    ) -> EffectiveRoutingRule {
        EffectiveRoutingRule {
            allow_cut_out: true,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: crate::ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode,
            request_compression_algorithm: RequestCompressionAlgorithm::Identity,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: Vec::new(),
            available_models_mode: AvailableModelsMode::Allowlist,
            available_models_defined: false,
            tag_available_models: None,
            status_change_reasons: default_status_change_reasons(),
            status_change_reason_field_sources: default_status_change_reason_field_sources("root"),
            system_denied_models: Vec::new(),
            source_tag_ids: Vec::new(),
            source_tag_names: Vec::new(),
            field_sources: EffectiveRoutingRuleFieldSources {
                allow_cut_out: "root".to_string(),
                allow_cut_in: "root".to_string(),
                priority_tier: "root".to_string(),
                fast_mode_rewrite_mode: "root".to_string(),
                image_tool_rewrite_mode: "root".to_string(),
                codex_imagegen_rewrite_mode: "root".to_string(),
                request_compression_algorithm: "root".to_string(),
                concurrency_limit: "root".to_string(),
                upstream_429_retry: "root".to_string(),
                available_models: "root".to_string(),
                available_models_mode: "root".to_string(),
                system_denied_models: "root".to_string(),
            },
            timeouts: RoutingTimeoutSettings::default(),
            timeout_field_sources: RoutingTimeoutFieldSources {
                responses_first_byte_timeout_secs: "root".to_string(),
                compact_first_byte_timeout_secs: "root".to_string(),
                image_first_byte_timeout_secs: "root".to_string(),
                responses_stream_timeout_secs: "root".to_string(),
                compact_stream_timeout_secs: "root".to_string(),
            },
        }
    }

    #[test]
    fn codex_non_default_rewrite_drops_hosted_image_tool_requirement() {
        let keep_original = request_capability_requirements_after_codex_imagegen_rewrite(
            "/v1/responses",
            crate::ImageIntent::Yes,
            Some("gpt-5.6-codex"),
            true,
            &effective_rule(crate::CodexImagegenRewriteMode::KeepOriginal),
        );
        let force_add = request_capability_requirements_after_codex_imagegen_rewrite(
            "/v1/responses",
            crate::ImageIntent::Yes,
            Some("gpt-5.6-codex"),
            true,
            &effective_rule(crate::CodexImagegenRewriteMode::ForceAdd),
        );

        assert!(keep_original.response_image_tool);
        assert!(!force_add.response_image_tool);
        assert!(force_add.codex_imagegen);
        assert!(force_add.response_endpoint);
        assert!(!account_accepts_request_capabilities(
            force_add,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Unsupported,
            CapabilitySupport::Unknown,
        ));
        assert!(account_accepts_request_capabilities(
            force_add,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            effective_capability_support(
                CapabilitySupport::Unsupported,
                Some(CapabilitySupport::Supported),
            ),
            CapabilitySupport::Unknown,
        ));

        let image_model = request_capability_requirements_after_codex_imagegen_rewrite(
            "/v1/responses",
            crate::ImageIntent::Yes,
            Some("gpt-image-1"),
            true,
            &effective_rule(crate::CodexImagegenRewriteMode::ForceAdd),
        );
        assert!(image_model.response_image_tool);
        assert!(image_model.codex_imagegen);

        let force_remove = request_capability_requirements_after_codex_imagegen_rewrite(
            "/v1/responses",
            crate::ImageIntent::Yes,
            Some("gpt-5.6-codex"),
            true,
            &effective_rule(crate::CodexImagegenRewriteMode::ForceRemove),
        );
        assert!(!force_remove.codex_imagegen);

        let fill_missing_without_image_intent =
            request_capability_requirements_after_codex_imagegen_rewrite(
                "/v1/responses",
                crate::ImageIntent::Unknown,
                Some("gpt-5.6-codex"),
                true,
                &effective_rule(crate::CodexImagegenRewriteMode::FillMissing),
            );
        assert!(!fill_missing_without_image_intent.codex_imagegen);
    }
}
