use super::*;
use std::collections::BTreeMap;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUpstreamAccountRequest {
    pub(crate) display_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) email: OptionalField<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) upstream_base_url: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) bound_proxy_keys: OptionalField<Vec<String>>,
    pub(crate) enabled: Option<bool>,
    pub(crate) is_mother: Option<bool>,
    pub(crate) api_key: Option<String>,
    pub(crate) local_primary_limit: Option<f64>,
    pub(crate) local_secondary_limit: Option<f64>,
    pub(crate) local_limit_unit: Option<String>,
    pub(crate) tag_ids: Option<Vec<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) response_endpoint_capability_override: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) chat_completions_capability_override: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) image_endpoint_capability_override: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) response_image_tool_capability_override: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) codex_imagegen_capability_override: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) standalone_search_capability_override: OptionalField<String>,
    pub(crate) routing_rule: Option<UpdateGroupAccountRoutingRuleRequest>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExternalAccountIdentity {
    pub(crate) client_id: String,
    pub(crate) source_account_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalOauthCredentialsRequest {
    pub(crate) email: String,
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: String,
    #[serde(default)]
    pub(crate) token_type: Option<String>,
    #[serde(default)]
    pub(crate) expired: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalUpstreamAccountMetadataRequest {
    pub(crate) display_name: Option<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) enabled: Option<bool>,
    pub(crate) is_mother: Option<bool>,
    #[serde(default)]
    pub(crate) tag_ids: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalUpstreamAccountUpsertRequest {
    #[serde(flatten)]
    pub(crate) metadata: ExternalUpstreamAccountMetadataRequest,
    pub(crate) oauth: ExternalOauthCredentialsRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalUpstreamAccountReloginRequest {
    pub(crate) oauth: ExternalOauthCredentialsRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTagRequest {
    pub(crate) name: String,
    pub(crate) allow_cut_out: bool,
    pub(crate) allow_cut_in: bool,
    pub(crate) priority_tier: Option<String>,
    pub(crate) fast_mode_rewrite_mode: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) upstream_429_retry_enabled: Option<bool>,
    pub(crate) upstream_429_max_retries: Option<u8>,
    #[serde(default)]
    pub(crate) available_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateTagRequest {
    pub(crate) name: Option<String>,
    pub(crate) allow_cut_out: Option<bool>,
    pub(crate) allow_cut_in: Option<bool>,
    pub(crate) priority_tier: Option<String>,
    pub(crate) fast_mode_rewrite_mode: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) upstream_429_retry_enabled: Option<bool>,
    pub(crate) upstream_429_max_retries: Option<u8>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) available_models: OptionalField<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateGroupAccountRoutingRuleRequest {
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) allow_cut_out: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) allow_cut_in: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) priority_tier: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) fast_mode_rewrite_mode: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) image_tool_rewrite_mode: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) codex_imagegen_rewrite_mode: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) request_compression_algorithm: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) oauth_request_compression_algorithm: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) concurrency_limit: OptionalField<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) upstream_429_retry_enabled: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) upstream_429_max_retries: OptionalField<u8>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) available_models: OptionalField<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) available_models_mode: OptionalField<String>,
    #[serde(default)]
    pub(crate) status_change_reasons: Option<UpdateStatusChangeReasonSettingsRequest>,
    #[serde(default)]
    pub(crate) timeouts: Option<UpdateRoutingTimeoutSettingsRequest>,
}

impl UpdateGroupAccountRoutingRuleRequest {
    pub(crate) fn priority_tier_value(&self) -> Option<&str> {
        match &self.priority_tier {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn fast_mode_rewrite_mode_value(&self) -> Option<&str> {
        match &self.fast_mode_rewrite_mode {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn image_tool_rewrite_mode_value(&self) -> Option<&str> {
        match &self.image_tool_rewrite_mode {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn codex_imagegen_rewrite_mode_value(&self) -> Option<&str> {
        match &self.codex_imagegen_rewrite_mode {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn request_compression_algorithm_value(&self) -> Option<&str> {
        match &self.request_compression_algorithm {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn oauth_request_compression_algorithm_value(&self) -> Option<&str> {
        match &self.oauth_request_compression_algorithm {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn status_change_reason_field(
        &self,
        reason_code: &str,
    ) -> Result<OptionalField<bool>> {
        self.status_change_reasons
            .as_ref()
            .map(|value| value.field(reason_code))
            .transpose()
            .map(|value| value.unwrap_or(OptionalField::Missing))
    }
}

pub(crate) fn optional_bool_to_i64(value: &OptionalField<bool>) -> Option<i64> {
    match value {
        OptionalField::Value(value) => Some(if *value { 1_i64 } else { 0_i64 }),
        OptionalField::Missing | OptionalField::Null => None,
    }
}

pub(crate) fn optional_retry_count_to_i64(value: &OptionalField<u8>) -> Option<i64> {
    match value {
        OptionalField::Value(value) => {
            Some(i64::from(normalize_group_upstream_429_max_retries(*value)))
        }
        OptionalField::Missing | OptionalField::Null => None,
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub(crate) struct UpdateStatusChangeReasonSettingsRequest {
    pub(crate) values: BTreeMap<String, Option<bool>>,
}

impl UpdateStatusChangeReasonSettingsRequest {
    pub(super) fn validate_keys(&self) -> Result<()> {
        for reason_code in self.values.keys() {
            match canonical_status_change_reason_code(reason_code) {
                Some(canonical) if canonical == reason_code => {}
                Some(_) => bail!(
                    "legacy status change reason keys are read-only; use canonical reasonCode values"
                ),
                None => bail!("unknown status change reason: {reason_code}"),
            }
        }
        Ok(())
    }

    fn field(&self, reason_code: &str) -> Result<OptionalField<bool>> {
        self.validate_keys()?;
        let Some(reason_code) = canonical_status_change_reason_code(reason_code) else {
            bail!("unknown status change reason: {reason_code}");
        };
        Ok(match self.values.get(reason_code) {
            Some(Some(value)) => OptionalField::Value(*value),
            Some(None) => OptionalField::Null,
            None => OptionalField::Missing,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListTagsQuery {
    pub(crate) search: Option<String>,
    pub(crate) has_accounts: Option<bool>,
    pub(crate) allow_cut_in: Option<bool>,
    pub(crate) allow_cut_out: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeysQuery {
    pub(crate) limit: Option<i64>,
    pub(crate) activity_hours: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUpstreamAccountGroupRequest {
    pub(crate) note: Option<String>,
    #[serde(default)]
    pub(crate) bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) single_account_rotation_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) upstream_429_retry_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) upstream_429_max_retries: Option<u8>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) routing_rule: Option<UpdateGroupAccountRoutingRuleRequest>,
}
