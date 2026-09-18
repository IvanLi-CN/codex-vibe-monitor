use super::*;

pub(crate) fn build_effective_routing_rule(tags: &[AccountTagSummary]) -> EffectiveRoutingRule {
    let inputs = TagRoutingRuleInputs::collect(tags);
    let field_source = if inputs.has_editable_tags {
        "tag"
    } else {
        "root"
    }
    .to_string();
    let available_models_source = if inputs.available_models_defined {
        "tag"
    } else {
        "root"
    }
    .to_string();
    let system_denied_models_source = if inputs.system_denied_models.is_empty() {
        "root"
    } else {
        "system"
    }
    .to_string();

    EffectiveRoutingRule {
        allow_cut_out: inputs.allow_cut_out,
        allow_cut_in: inputs.allow_cut_in,
        priority_tier: inputs.priority_tier,
        fast_mode_rewrite_mode: inputs.fast_mode_rewrite_mode,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        concurrency_limit: inputs.concurrency_limit,
        upstream_429_retry_enabled: inputs.upstream_429_retry_enabled,
        upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(
            inputs.upstream_429_retry_enabled,
            inputs.upstream_429_max_retries,
        ),
        available_models: inputs.available_models.unwrap_or_default(),
        available_models_mode: AvailableModelsMode::Allowlist,
        available_models_defined: inputs.available_models_defined,
        tag_available_models: None,
        status_change_reasons: default_status_change_reasons(),
        status_change_reason_field_sources: default_status_change_reason_field_sources("root"),
        system_denied_models: inputs.system_denied_models.into_iter().collect(),
        source_tag_ids: inputs.source_tag_ids,
        source_tag_names: inputs.source_tag_names,
        field_sources: EffectiveRoutingRuleFieldSources {
            allow_cut_out: field_source.clone(),
            allow_cut_in: field_source.clone(),
            priority_tier: field_source.clone(),
            fast_mode_rewrite_mode: field_source.clone(),
            image_tool_rewrite_mode: "root".to_string(),
            codex_imagegen_rewrite_mode: "root".to_string(),
            request_compression_algorithm: "root".to_string(),
            concurrency_limit: field_source.clone(),
            upstream_429_retry: field_source.clone(),
            available_models: available_models_source,
            available_models_mode: "root".to_string(),
            system_denied_models: system_denied_models_source,
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

struct TagRoutingRuleInputs {
    source_tag_ids: Vec<i64>,
    source_tag_names: Vec<String>,
    has_editable_tags: bool,
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: TagPriorityTier,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
    concurrency_limit: i64,
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
    available_models: Option<Vec<String>>,
    available_models_defined: bool,
    system_denied_models: BTreeSet<String>,
}

impl TagRoutingRuleInputs {
    fn collect(tags: &[AccountTagSummary]) -> Self {
        let has_editable_tags = tags
            .iter()
            .any(|tag| !tag.protected && tag.system_key.is_none());
        let mut inputs = Self {
            source_tag_ids: Vec::with_capacity(tags.len()),
            source_tag_names: Vec::with_capacity(tags.len()),
            has_editable_tags,
            allow_cut_out: true,
            allow_cut_in: true,
            priority_tier: if has_editable_tags {
                TagPriorityTier::Primary
            } else {
                TagPriorityTier::Normal
            },
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: None,
            available_models_defined: false,
            system_denied_models: BTreeSet::new(),
        };

        for tag in tags {
            inputs.source_tag_ids.push(tag.id);
            inputs.source_tag_names.push(tag.name.clone());
            inputs.apply_editable_tag_policy(tag);
            inputs.apply_available_models(tag);
            inputs.collect_system_denied_model(tag);
        }

        inputs
    }

    fn apply_editable_tag_policy(&mut self, tag: &AccountTagSummary) {
        if tag.protected || tag.system_key.is_some() {
            return;
        }
        self.allow_cut_out &= tag.routing_rule.allow_cut_out;
        self.allow_cut_in &= tag.routing_rule.allow_cut_in;
        self.priority_tier = self.priority_tier.min(tag.routing_rule.priority_tier);
        if tag.routing_rule.fast_mode_rewrite_mode.merge_rank()
            < self.fast_mode_rewrite_mode.merge_rank()
        {
            self.fast_mode_rewrite_mode = tag.routing_rule.fast_mode_rewrite_mode;
        }
        self.concurrency_limit =
            merge_concurrency_limits(self.concurrency_limit, tag.routing_rule.concurrency_limit);
        if tag.routing_rule.upstream_429_retry_enabled {
            self.upstream_429_retry_enabled = true;
            self.upstream_429_max_retries = self
                .upstream_429_max_retries
                .max(tag.routing_rule.upstream_429_max_retries);
        }
    }

    fn apply_available_models(&mut self, tag: &AccountTagSummary) {
        if tag.available_models_invalid {
            self.available_models_defined = true;
            self.available_models = Some(Vec::new());
        } else if !tag.routing_rule.available_models.is_empty() {
            self.available_models_defined = true;
            self.available_models = Some(match self.available_models.take() {
                Some(current) => {
                    intersect_available_models(current, &tag.routing_rule.available_models)
                }
                None => tag.routing_rule.available_models.clone(),
            });
        }
    }

    fn collect_system_denied_model(&mut self, tag: &AccountTagSummary) {
        if let Some(model) = tag
            .system_key
            .as_deref()
            .and_then(|value| value.strip_prefix("unsupported_model:"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            self.system_denied_models.insert(model.to_string());
        }
    }
}
