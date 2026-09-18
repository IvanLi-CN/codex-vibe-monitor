fn invalid_imported_oauth_validation_row(
    item: &ImportOauthCredentialFileRequest,
    detail: String,
) -> ImportedOauthValidationRow {
    ImportedOauthValidationRow {
        source_id: item.source_id.clone(),
        file_name: item.file_name.clone(),
        email: None,
        chatgpt_account_id: None,
        chatgpt_user_id: None,
        display_name: None,
        token_expires_at: None,
        matched_account: None,
        status: IMPORT_VALIDATION_STATUS_INVALID.to_string(),
        detail: Some(detail),
        attempts: 0,
    }
}

fn imported_oauth_validation_row(
    normalized: NormalizedImportedOauthCredentials,
    matched_account: Option<ImportedOauthMatchSummary>,
    status: &str,
    detail: Option<String>,
) -> ImportedOauthValidationRow {
    ImportedOauthValidationRow {
        source_id: normalized.source_id,
        file_name: normalized.file_name,
        email: Some(normalized.email),
        chatgpt_account_id: Some(normalized.chatgpt_account_id),
        chatgpt_user_id: normalized.chatgpt_user_id,
        display_name: Some(normalized.display_name),
        token_expires_at: Some(normalized.token_expires_at),
        matched_account,
        status: status.to_string(),
        detail,
        attempts: 0,
    }
}

async fn build_imported_oauth_validation_result_with_reservation(
    state: &AppState,
    normalized: NormalizedImportedOauthCredentials,
    matched_account: Option<ImportedOauthMatchSummary>,
    refresh_scope: &ForwardProxyRouteScope,
    usage_scope: &ForwardProxyRouteScope,
) -> Result<(
    ImportedOauthValidationRow,
    Option<ImportedOauthValidatedImportData>,
)> {
    let reservation_key = reserve_imported_oauth_node_shunt_scope(
        state,
        &normalized.source_id,
        matched_account.as_ref().map(|account| account.account_id),
        usage_scope,
    )?;
    let result = build_imported_oauth_validation_result(
        state,
        normalized,
        matched_account,
        refresh_scope,
        usage_scope,
    )
    .await;
    release_imported_oauth_node_shunt_scope(state, reservation_key);
    Ok(result)
}

struct ImportedOauthValidationJobContext<'a> {
    state: &'a AppState,
    job: &'a Arc<ImportedOauthValidationJob>,
    binding: &'a ResolvedRequiredGroupProxyBinding,
    assignments: &'a UpstreamAccountNodeShuntAssignments,
    refresh_scope: &'a ForwardProxyRouteScope,
}

async fn run_imported_oauth_validation_job(
    state: &AppState,
    binding: &ResolvedRequiredGroupProxyBinding,
    items: &[ImportOauthCredentialFileRequest],
    job: &Arc<ImportedOauthValidationJob>,
) -> Result<(), String> {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .map_err(|err| err.to_string())?;
    let refresh_scope = required_account_forward_proxy_scope(
        Some(&binding.group_name),
        binding.bound_proxy_keys.clone(),
    )
    .map_err(|err| err.to_string())?;
    let context = ImportedOauthValidationJobContext {
        state,
        job,
        binding,
        assignments: &assignments,
        refresh_scope: &refresh_scope,
    };
    let mut seen_keys = HashSet::new();
    let mut consumed_proxy_keys = HashSet::new();
    for (row_index, item) in items.iter().enumerate() {
        if job.cancel.is_cancelled() {
            finish_imported_oauth_validation_job_cancelled(job).await;
            return Ok(());
        }
        process_imported_oauth_validation_job_item(
            &context,
            row_index,
            item,
            &mut seen_keys,
            &mut consumed_proxy_keys,
        )
        .await?;
    }
    if job.cancel.is_cancelled() {
        finish_imported_oauth_validation_job_cancelled(job).await;
    } else {
        finish_imported_oauth_validation_job_completed(job).await;
    }
    Ok(())
}

async fn update_imported_oauth_validation_job_error(
    job: &Arc<ImportedOauthValidationJob>,
    row_index: usize,
    normalized: NormalizedImportedOauthCredentials,
    matched_account: Option<ImportedOauthMatchSummary>,
    detail: String,
) {
    update_imported_oauth_validation_job_row(
        job,
        row_index,
        imported_oauth_validation_row(
            normalized,
            matched_account,
            IMPORT_VALIDATION_STATUS_ERROR,
            Some(detail),
        ),
        None,
    )
    .await;
}

async fn update_imported_oauth_validation_job_invalid(
    job: &Arc<ImportedOauthValidationJob>,
    row_index: usize,
    item: &ImportOauthCredentialFileRequest,
    detail: String,
) {
    update_imported_oauth_validation_job_row(
        job,
        row_index,
        invalid_imported_oauth_validation_row(item, detail),
        None,
    )
    .await;
}

fn invalid_imported_oauth_import_result(
    item: ImportOauthCredentialFileRequest,
    detail: String,
) -> ImportedOauthImportResult {
    ImportedOauthImportResult {
        source_id: item.source_id,
        file_name: item.file_name,
        email: None,
        chatgpt_account_id: None,
        account_id: None,
        status: IMPORT_RESULT_STATUS_FAILED.to_string(),
        detail: Some(detail),
        matched_account: None,
    }
}

fn failed_imported_oauth_import_result(
    normalized: NormalizedImportedOauthCredentials,
    account_id: Option<i64>,
    matched_account: Option<ImportedOauthMatchSummary>,
    detail: String,
) -> ImportedOauthImportResult {
    ImportedOauthImportResult {
        source_id: normalized.source_id,
        file_name: normalized.file_name,
        email: Some(normalized.email),
        chatgpt_account_id: Some(normalized.chatgpt_account_id),
        account_id,
        status: IMPORT_RESULT_STATUS_FAILED.to_string(),
        detail: Some(detail),
        matched_account,
    }
}

struct ImportedOauthCreatePlan {
    group_name: Option<String>,
    tag_ids: Vec<i64>,
    requested_group_metadata_changes: RequestedGroupMetadataChanges,
}

struct ImportBatch {
    input_files: usize,
    selected_files: usize,
    created: usize,
    updated_existing: usize,
    failed: usize,
    results: Vec<ImportedOauthImportResult>,
}

fn finalize_import_batch(batch: ImportBatch) -> Json<ImportedOauthImportResponse> {
    Json(batch.into_response())
}

fn imported_oauth_import_success_result(
    normalized: NormalizedImportedOauthCredentials,
    account_id: i64,
    existing: bool,
    detail: Option<String>,
    matched_account: Option<ImportedOauthMatchSummary>,
) -> ImportedOauthImportResult {
    ImportedOauthImportResult {
        source_id: normalized.source_id,
        file_name: normalized.file_name,
        email: Some(normalized.email),
        chatgpt_account_id: Some(normalized.chatgpt_account_id),
        account_id: Some(account_id),
        status: if existing {
            IMPORT_RESULT_STATUS_UPDATED_EXISTING.to_string()
        } else {
            IMPORT_RESULT_STATUS_CREATED.to_string()
        },
        detail,
        matched_account,
    }
}

fn normalize_imported_oauth_for_import(
    cached_validation: Option<&ImportedOauthValidatedImportData>,
    item: &ImportOauthCredentialFileRequest,
) -> Result<NormalizedImportedOauthCredentials, String> {
    cached_validation
        .map(|cached| Ok(cached.normalized.clone()))
        .unwrap_or_else(|| normalize_imported_oauth_credentials(item))
}

async fn load_imported_oauth_existing_match(
    state: &AppState,
    normalized: &NormalizedImportedOauthCredentials,
) -> Result<Option<UpstreamAccountRow>, String> {
    find_existing_import_match(
        &state.pool,
        normalized.chatgpt_user_id.as_deref(),
        &normalized.chatgpt_account_id,
        &normalized.email,
    )
    .await
    .map_err(|err| err.to_string())
}

impl ImportBatch {
    fn new(input_files: usize, selected_files: usize) -> Self {
        Self {
            input_files,
            selected_files,
            created: 0,
            updated_existing: 0,
            failed: 0,
            results: Vec::new(),
        }
    }

    fn record_failure(&mut self, result: ImportedOauthImportResult) {
        self.failed += 1;
        self.results.push(result);
    }

    fn record_success(&mut self, existing: bool, result: ImportedOauthImportResult) {
        if existing {
            self.updated_existing += 1;
        } else {
            self.created += 1;
        }
        self.results.push(result);
    }

    fn into_response(self) -> ImportedOauthImportResponse {
        ImportedOauthImportResponse {
            summary: ImportedOauthImportSummary {
                input_files: self.input_files,
                selected_files: self.selected_files,
                created: self.created,
                updated_existing: self.updated_existing,
                failed: self.failed,
            },
            results: self.results,
        }
    }
}

struct PreparedImportedOauthImport {
    crypto_key: [u8; 32],
    items: Vec<ImportOauthCredentialFileRequest>,
    selected_source_ids: HashSet<String>,
    cached_validation_results: HashMap<String, ImportedOauthValidatedImportData>,
    resolved_group_binding: ResolvedRequiredGroupProxyBinding,
    create_plan: ImportedOauthCreatePlan,
    assignments: UpstreamAccountNodeShuntAssignments,
    refresh_scope: ForwardProxyRouteScope,
}

async fn prepare_imported_oauth_import(
    state: &AppState,
    payload: ImportValidatedOauthAccountsRequest,
) -> Result<PreparedImportedOauthImport, (StatusCode, String)> {
    let ImportValidatedOauthAccountsRequest {
        items,
        selected_source_ids,
        validation_job_id,
        group_name,
        group_bound_proxy_keys,
        group_node_shunt_enabled,
        group_single_account_rotation_enabled,
        group_note,
        concurrency_limit,
        tag_ids,
    } = payload;
    let crypto_key = *state.upstream_accounts.require_crypto_key()?;
    let selected_source_ids = selected_source_ids
        .into_iter()
        .filter_map(|value| normalize_optional_text(Some(value)))
        .collect::<HashSet<_>>();
    if selected_source_ids.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "selectedSourceIds must not be empty".to_string(),
        ));
    }
    let group_name = normalize_optional_text(group_name);
    let group_note = normalize_optional_text(group_note);
    let group_concurrency_limit =
        normalize_concurrency_limit(concurrency_limit, "concurrencyLimit")?;
    validate_group_note_target(group_name.as_deref(), group_note.is_some())?;
    let requested_group_metadata_changes =
        build_requested_group_metadata_changes(RequestedGroupMetadataInput::from_import_values(
            group_note,
            group_bound_proxy_keys.clone(),
            group_concurrency_limit,
            concurrency_limit.is_some(),
            group_node_shunt_enabled,
            group_single_account_rotation_enabled,
        ));
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state,
        group_name,
        group_bound_proxy_keys,
        group_node_shunt_enabled,
    )
    .await?;
    reject_manual_tag_ids(&tag_ids)?;
    let cached_validation_results = load_cached_imported_oauth_validation_results(
        state,
        validation_job_id,
        &resolved_group_binding,
    )
    .await;
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .map_err(internal_error_tuple)?;
    let refresh_scope = required_account_forward_proxy_scope(
        Some(&resolved_group_binding.group_name),
        resolved_group_binding.bound_proxy_keys.clone(),
    )
    .map_err(internal_error_tuple)?;
    Ok(PreparedImportedOauthImport {
        crypto_key,
        items,
        selected_source_ids,
        cached_validation_results,
        create_plan: ImportedOauthCreatePlan {
            group_name: Some(resolved_group_binding.group_name.clone()),
            tag_ids: Vec::new(),
            requested_group_metadata_changes,
        },
        resolved_group_binding,
        assignments,
        refresh_scope,
    })
}

async fn load_cached_imported_oauth_validation_results(
    state: &AppState,
    validation_job_id: Option<String>,
    binding: &ResolvedRequiredGroupProxyBinding,
) -> HashMap<String, ImportedOauthValidatedImportData> {
    let Some(job_id) = normalize_optional_text(validation_job_id) else {
        return HashMap::new();
    };
    let Some(job) = state.upstream_accounts.get_validation_job(&job_id).await else {
        return HashMap::new();
    };
    if job.target_group_name != binding.group_name
        || job.target_bound_proxy_keys != binding.bound_proxy_keys
        || job.target_node_shunt_enabled != binding.node_shunt_enabled
    {
        return HashMap::new();
    }
    job.validated_imports.lock().await.clone()
}

async fn persist_imported_oauth_account(
    state: Arc<AppState>,
    crypto_key: &[u8; 32],
    existing_row: Option<&UpstreamAccountRow>,
    normalized: &NormalizedImportedOauthCredentials,
    probe: &ImportedOauthProbeOutcome,
    plan: &ImportedOauthCreatePlan,
) -> Result<(i64, Option<String>), (StatusCode, String)> {
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(probe.credentials.clone()),
    )
    .map_err(internal_error_tuple)?;
    if let Some(existing_row) = existing_row {
        let warning = state
            .upstream_accounts
            .account_ops
            .run_persist_imported_oauth(state.clone(), existing_row.id, probe.clone())
            .await?;
        return Ok((existing_row.id, warning));
    }
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    ensure_display_name_available_for_oauth_identity(
        &mut *tx,
        &normalized.display_name,
        None,
        probe.claims.chatgpt_account_id.as_deref(),
        probe.claims.chatgpt_user_id.as_deref(),
        plan.group_name.as_deref(),
        probe.claims.chatgpt_plan_type.as_deref(),
    )
    .await?;
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: &normalized.display_name,
            chosen_email: Some(normalized.email.clone()),
            verified_email: normalize_email_value(probe.claims.email.clone()),
            group_name: plan.group_name.clone(),
            is_mother: false,
            note: None,
            tag_ids: plan.tag_ids.clone(),
            requested_group_metadata_changes: plan.requested_group_metadata_changes.clone(),
            claims: &probe.claims,
            encrypted_credentials,
            has_refresh_token: oauth_credentials_have_refresh_token(&probe.credentials),
            token_expires_at: &probe.token_expires_at,
            external_identity: None,
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;
    let warning = state
        .upstream_accounts
        .account_ops
        .run_persist_imported_oauth(state.clone(), account_id, probe.clone())
        .await?;
    publish_new_account_routing_availability_if_selectable(state.as_ref(), account_id).await;
    Ok((account_id, warning))
}

async fn process_imported_oauth_validation_job_item(
    context: &ImportedOauthValidationJobContext<'_>,
    row_index: usize,
    item: &ImportOauthCredentialFileRequest,
    seen_keys: &mut HashSet<String>,
    consumed_proxy_keys: &mut HashSet<String>,
) -> Result<(), String> {
    let normalized = match normalize_imported_oauth_credentials(item) {
        Ok(value) => value,
        Err(message) => {
            update_imported_oauth_validation_job_invalid(context.job, row_index, item, message)
                .await;
            return Ok(());
        }
    };
    let match_key = imported_match_key(
        normalized.chatgpt_user_id.as_deref(),
        &normalized.email,
        &normalized.chatgpt_account_id,
    );
    if !seen_keys.insert(match_key) {
        update_imported_oauth_validation_job_row(
            context.job,
            row_index,
            imported_oauth_validation_row(
                normalized,
                None,
                IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT,
                Some("duplicate credential in current import selection".to_string()),
            ),
            None,
        )
        .await;
        return Ok(());
    }
    let existing_match = match find_existing_import_match(
        &context.state.pool,
        normalized.chatgpt_user_id.as_deref(),
        &normalized.chatgpt_account_id,
        &normalized.email,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            update_imported_oauth_validation_job_error(
                context.job,
                row_index,
                normalized,
                None,
                err.to_string(),
            )
            .await;
            return Ok(());
        }
    };
    let matched_account = existing_match.as_ref().map(import_match_summary_from_row);
    let usage_scope = match resolve_imported_oauth_probe_scope(
        context.state,
        context.binding,
        Some(context.assignments),
        existing_match.as_ref(),
        consumed_proxy_keys,
    )
    .await
    {
        Ok(scope) => scope,
        Err(err) => {
            update_imported_oauth_validation_job_error(
                context.job,
                row_index,
                normalized,
                matched_account,
                err.to_string(),
            )
            .await;
            return Ok(());
        }
    };
    let (row, validated_import) = build_imported_oauth_validation_result_with_reservation(
        context.state,
        normalized,
        matched_account,
        context.refresh_scope,
        &usage_scope,
    )
    .await
    .map_err(|err| err.to_string())?;
    if let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = &usage_scope
        && validated_import.is_some()
    {
        consumed_proxy_keys.insert(proxy_key.clone());
    }
    update_imported_oauth_validation_job_row(context.job, row_index, row, validated_import).await;
    Ok(())
}
