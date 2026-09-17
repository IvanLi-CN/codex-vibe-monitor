#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatusRequest {
    #[serde(default)]
    pub(crate) session_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateApiKeyAccountRequest {
    pub(crate) display_name: String,
    pub(crate) email: Option<String>,
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
    pub(crate) bound_proxy_keys: OptionalField<Vec<String>>,
    pub(crate) upstream_base_url: Option<String>,
    pub(crate) api_key: String,
    pub(crate) is_mother: Option<bool>,
    pub(crate) local_primary_limit: Option<f64>,
    pub(crate) local_secondary_limit: Option<f64>,
    pub(crate) local_limit_unit: Option<String>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfirmApiKeyGroupMigrationRequest {
    pub(crate) confirmation_hash: String,
    #[serde(default)]
    pub(crate) disabled_strategies: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiKeyGroupMigrationPreflightResponse {
    pub(crate) confirmation_hash: String,
    pub(crate) api_key_count: usize,
    pub(crate) portable_fields: Vec<String>,
    pub(crate) blocked_strategies: Vec<String>,
    pub(crate) can_migrate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiKeyGroupMigrationResponse {
    pub(crate) migrated_count: usize,
    pub(crate) confirmation_hash: String,
    pub(crate) audit_action: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportOauthCredentialFileRequest {
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidateImportedOauthAccountsRequest {
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) items: Vec<ImportOauthCredentialFileRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportValidatedOauthAccountsRequest {
    #[serde(default)]
    pub(crate) items: Vec<ImportOauthCredentialFileRequest>,
    #[serde(default)]
    pub(crate) selected_source_ids: Vec<String>,
    #[serde(default)]
    pub(crate) validation_job_id: Option<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthMatchSummary {
    pub(crate) account_id: i64,
    pub(crate) display_name: String,
    pub(crate) group_name: Option<String>,
    pub(crate) status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationRow {
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) token_expires_at: Option<String>,
    pub(crate) matched_account: Option<ImportedOauthMatchSummary>,
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
    pub(crate) attempts: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationResponse {
    pub(crate) input_files: usize,
    pub(crate) unique_in_input: usize,
    pub(crate) duplicate_in_input: usize,
    pub(crate) rows: Vec<ImportedOauthValidationRow>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationCounts {
    pub(crate) pending: usize,
    pub(crate) duplicate_in_input: usize,
    pub(crate) ok: usize,
    pub(crate) ok_exhausted: usize,
    pub(crate) invalid: usize,
    pub(crate) error: usize,
    pub(crate) checked: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationJobResponse {
    pub(crate) job_id: String,
    pub(crate) snapshot: ImportedOauthValidationResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationSnapshotEvent {
    pub(crate) snapshot: ImportedOauthValidationResponse,
    pub(crate) counts: ImportedOauthValidationCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationRowEvent {
    pub(crate) row: ImportedOauthValidationRow,
    pub(crate) counts: ImportedOauthValidationCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationFailedEvent {
    pub(crate) snapshot: ImportedOauthValidationResponse,
    pub(crate) counts: ImportedOauthValidationCounts,
    pub(crate) error: String,
}

#[derive(Debug, Clone)]
pub(crate) enum ImportedOauthValidationTerminalEvent {
    Completed(ImportedOauthValidationSnapshotEvent),
    Failed(ImportedOauthValidationFailedEvent),
    Cancelled(ImportedOauthValidationSnapshotEvent),
}

#[derive(Debug, Clone)]
pub(crate) enum ImportedOauthValidationJobEvent {
    Row(ImportedOauthValidationRowEvent),
    Completed(ImportedOauthValidationSnapshotEvent),
    Failed(ImportedOauthValidationFailedEvent),
    Cancelled(ImportedOauthValidationSnapshotEvent),
}

#[derive(Debug)]
pub(crate) struct ImportedOauthValidationJob {
    pub(crate) target_group_name: String,
    pub(crate) target_bound_proxy_keys: Vec<String>,
    pub(crate) target_node_shunt_enabled: bool,
    pub(crate) snapshot: Mutex<ImportedOauthValidationResponse>,
    pub(crate) validated_imports: Mutex<HashMap<String, ImportedOauthValidatedImportData>>,
    pub(crate) broadcaster: broadcast::Sender<ImportedOauthValidationJobEvent>,
    pub(crate) cancel: CancellationToken,
    pub(crate) terminal_event: Mutex<Option<ImportedOauthValidationTerminalEvent>>,
}

impl ImportedOauthValidationJob {
    pub(crate) fn new(
        snapshot: ImportedOauthValidationResponse,
        binding: &ResolvedRequiredGroupProxyBinding,
    ) -> Self {
        let (broadcaster, _rx) = broadcast::channel(256);
        Self {
            target_group_name: binding.group_name.clone(),
            target_bound_proxy_keys: binding.bound_proxy_keys.clone(),
            target_node_shunt_enabled: binding.node_shunt_enabled,
            snapshot: Mutex::new(snapshot),
            validated_imports: Mutex::new(HashMap::new()),
            broadcaster,
            cancel: CancellationToken::new(),
            terminal_event: Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountActionRequest {
    pub(crate) account_ids: Vec<i64>,
    pub(crate) action: String,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountActionResponse {
    pub(crate) action: String,
    pub(crate) requested_count: usize,
    pub(crate) completed_count: usize,
    pub(crate) succeeded_count: usize,
    pub(crate) failed_count: usize,
    pub(crate) results: Vec<BulkUpstreamAccountActionResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountActionResult {
    pub(crate) account_id: i64,
    pub(crate) display_name: Option<String>,
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncJobRequest {
    pub(crate) account_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncCounts {
    pub(crate) total: usize,
    pub(crate) completed: usize,
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncRow {
    pub(crate) account_id: i64,
    pub(crate) display_name: String,
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncSnapshot {
    pub(crate) job_id: String,
    pub(crate) status: String,
    pub(crate) rows: Vec<BulkUpstreamAccountSyncRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncJobResponse {
    pub(crate) job_id: String,
    pub(crate) snapshot: BulkUpstreamAccountSyncSnapshot,
    pub(crate) counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncRowEvent {
    pub(crate) row: BulkUpstreamAccountSyncRow,
    pub(crate) counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncSnapshotEvent {
    pub(crate) snapshot: BulkUpstreamAccountSyncSnapshot,
    pub(crate) counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncFailedEvent {
    pub(crate) snapshot: BulkUpstreamAccountSyncSnapshot,
    pub(crate) counts: BulkUpstreamAccountSyncCounts,
    pub(crate) error: String,
}

#[derive(Debug, Clone)]
pub(crate) enum BulkUpstreamAccountSyncTerminalEvent {
    Completed(BulkUpstreamAccountSyncSnapshotEvent),
    Failed(BulkUpstreamAccountSyncFailedEvent),
    Cancelled(BulkUpstreamAccountSyncSnapshotEvent),
}

#[derive(Debug, Clone)]
pub(crate) enum BulkUpstreamAccountSyncJobEvent {
    Row(BulkUpstreamAccountSyncRowEvent),
    Completed(BulkUpstreamAccountSyncSnapshotEvent),
    Failed(BulkUpstreamAccountSyncFailedEvent),
    Cancelled(BulkUpstreamAccountSyncSnapshotEvent),
}

#[derive(Debug)]
pub(crate) struct BulkUpstreamAccountSyncJob {
    pub(crate) snapshot: Mutex<BulkUpstreamAccountSyncSnapshot>,
    pub(crate) broadcaster: broadcast::Sender<BulkUpstreamAccountSyncJobEvent>,
    pub(crate) cancel: CancellationToken,
    pub(crate) terminal_event: Mutex<Option<BulkUpstreamAccountSyncTerminalEvent>>,
}

impl BulkUpstreamAccountSyncJob {
    pub(crate) fn new(snapshot: BulkUpstreamAccountSyncSnapshot) -> Self {
        let (broadcaster, _rx) = broadcast::channel(256);
        Self {
            snapshot: Mutex::new(snapshot),
            broadcaster,
            cancel: CancellationToken::new(),
            terminal_event: Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthImportResult {
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) account_id: Option<i64>,
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
    pub(crate) matched_account: Option<ImportedOauthMatchSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthImportSummary {
    pub(crate) input_files: usize,
    pub(crate) selected_files: usize,
    pub(crate) created: usize,
    pub(crate) updated_existing: usize,
    pub(crate) failed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthImportResponse {
    pub(crate) summary: ImportedOauthImportSummary,
    pub(crate) results: Vec<ImportedOauthImportResult>,
}

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
    fn validate_keys(&self) -> Result<()> {
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

#[derive(Debug, Deserialize)]
pub(crate) struct OauthCallbackQuery {
    pub(crate) code: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) error_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredApiKeyCredentials {
    pub(crate) api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredOauthCredentials {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: String,
    pub(crate) token_type: Option<String>,
}

pub(crate) fn normalize_oauth_refresh_token(value: Option<String>) -> Option<String> {
    normalize_optional_text(value)
}

pub(crate) fn oauth_refresh_token(credentials: &StoredOauthCredentials) -> Option<&str> {
    credentials
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn oauth_credentials_have_refresh_token(credentials: &StoredOauthCredentials) -> bool {
    oauth_refresh_token(credentials).is_some()
}

pub(crate) fn apply_oauth_token_response(
    credentials: &mut StoredOauthCredentials,
    response: OAuthTokenResponse,
) -> String {
    credentials.access_token = response.access_token;
    if let Some(refresh_token) = response.refresh_token {
        credentials.refresh_token = normalize_oauth_refresh_token(Some(refresh_token));
    }
    if let Some(id_token) = response.id_token {
        credentials.id_token = id_token;
    }
    credentials.token_type = response.token_type;
    format_utc_iso(Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum StoredCredentials {
    ApiKey(StoredApiKeyCredentials),
    Oauth(StoredOauthCredentials),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EncryptedCredentialsPayload {
    pub(crate) v: u8,
    pub(crate) nonce: String,
    pub(crate) ciphertext: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedUsageSnapshot {
    pub(crate) plan_type: Option<String>,
    pub(crate) limit_id: String,
    pub(crate) limit_name: Option<String>,
    pub(crate) primary: Option<NormalizedUsageWindow>,
    pub(crate) secondary: Option<NormalizedUsageWindow>,
    pub(crate) credits: Option<CreditsSnapshot>,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedUsageWindow {
    pub(crate) used_percent: f64,
    pub(crate) window_duration_mins: i64,
    pub(crate) resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthTokenResponse {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    #[serde(default)]
    pub(crate) id_token: Option<String>,
    #[serde(default)]
    pub(crate) token_type: Option<String>,
    pub(crate) expires_in: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ChatgptJwtClaims {
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_plan_type: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImportedOauthCredentialsFile {
    #[serde(rename = "type")]
    #[serde(default)]
    pub(crate) _source_type: Option<serde_json::Value>,
    pub(crate) email: String,
    pub(crate) account_id: String,
    #[serde(default)]
    pub(crate) expired: Option<String>,
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<serde_json::Value>,
    pub(crate) id_token: String,
    #[serde(default)]
    #[serde(rename = "last_refresh")]
    pub(crate) _last_refresh: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) token_type: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedImportedOauthCredentials {
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) email: String,
    pub(crate) display_name: String,
    pub(crate) chatgpt_account_id: String,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) token_expires_at: String,
    pub(crate) credentials: StoredOauthCredentials,
    pub(crate) claims: ChatgptJwtClaims,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedOauthProbeOutcome {
    pub(crate) token_expires_at: String,
    pub(crate) credentials: StoredOauthCredentials,
    pub(crate) claims: ChatgptJwtClaims,
    pub(crate) usage_snapshot: Option<NormalizedUsageSnapshot>,
    pub(crate) maintenance_proxy_snapshot: Option<AccountMaintenanceProxySnapshot>,
    pub(crate) exhausted: bool,
    pub(crate) usage_snapshot_warning: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedOauthValidatedImportData {
    pub(crate) normalized: NormalizedImportedOauthCredentials,
    pub(crate) probe: ImportedOauthProbeOutcome,
}

pub(crate) struct PersistOauthCallbackInput {
    pub(crate) session: OauthLoginSessionRow,
    pub(crate) display_name: String,
    pub(crate) chosen_email: Option<String>,
    pub(crate) verified_email: Option<String>,
    pub(crate) claims: ChatgptJwtClaims,
    pub(crate) encrypted_credentials: String,
    pub(crate) has_refresh_token: bool,
    pub(crate) token_expires_at: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtOuterClaims {
    #[serde(default)]
    pub(crate) email: Option<String>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    pub(crate) profile: Option<ChatgptJwtProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    pub(crate) auth: Option<ChatgptJwtAuthClaims>,
}
