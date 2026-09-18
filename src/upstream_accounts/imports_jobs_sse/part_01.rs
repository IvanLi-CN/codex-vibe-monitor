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
