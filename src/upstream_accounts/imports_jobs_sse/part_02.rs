pub(crate) async fn build_imported_oauth_validation_response(
    state: &AppState,
    items: &[ImportOauthCredentialFileRequest],
    binding: &ResolvedRequiredGroupProxyBinding,
) -> Result<ImportedOauthValidationResponse> {
    let mut seen_keys = HashSet::new();
    let mut consumed_proxy_keys = HashSet::new();
    let mut rows = Vec::with_capacity(items.len());
    let assignments = if binding.node_shunt_enabled {
        Some(build_upstream_account_node_shunt_assignments(state).await?)
    } else {
        None
    };
    let refresh_scope = required_account_forward_proxy_scope(
        Some(&binding.group_name),
        binding.bound_proxy_keys.clone(),
    )
    .expect("validated group binding should always resolve refresh scope");
    for item in items {
        let normalized = match normalize_imported_oauth_credentials(item) {
            Ok(value) => value,
            Err(message) => {
                rows.push(invalid_imported_oauth_validation_row(item, message));
                continue;
            }
        };
        let match_key = imported_match_key(
            normalized.chatgpt_user_id.as_deref(),
            &normalized.email,
            &normalized.chatgpt_account_id,
        );
        if !seen_keys.insert(match_key) {
            rows.push(imported_oauth_validation_row(
                normalized,
                None,
                IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT,
                Some("duplicate credential in current import selection".to_string()),
            ));
            continue;
        }
        let existing_match = match load_imported_oauth_existing_match(state, &normalized).await {
            Ok(value) => value,
            Err(err) => {
                rows.push(imported_oauth_validation_row(
                    normalized,
                    None,
                    IMPORT_VALIDATION_STATUS_ERROR,
                    Some(err.to_string()),
                ));
                continue;
            }
        };
        let matched_account = existing_match.as_ref().map(import_match_summary_from_row);
        let usage_scope = match resolve_imported_oauth_probe_scope(
            state,
            binding,
            assignments.as_ref(),
            existing_match.as_ref(),
            &consumed_proxy_keys,
        )
        .await
        {
            Ok(scope) => scope,
            Err(err) => {
                rows.push(imported_oauth_validation_row(
                    normalized,
                    matched_account,
                    IMPORT_VALIDATION_STATUS_ERROR,
                    Some(err.to_string()),
                ));
                continue;
            }
        };
        let (row, validated_import) = build_imported_oauth_validation_result_with_reservation(
            state,
            normalized,
            matched_account,
            &refresh_scope,
            &usage_scope,
        )
        .await?;
        if let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = &usage_scope
            && validated_import.is_some()
        {
            consumed_proxy_keys.insert(proxy_key.clone());
        }
        rows.push(row);
    }
    Ok(build_imported_oauth_validation_response_from_rows(
        items.len(),
        rows,
    ))
}

async fn resolve_imported_oauth_probe_scope(
    state: &AppState,
    binding: &ResolvedRequiredGroupProxyBinding,
    assignments: Option<&UpstreamAccountNodeShuntAssignments>,
    existing_match: Option<&UpstreamAccountRow>,
    consumed_proxy_keys: &HashSet<String>,
) -> Result<ForwardProxyRouteScope> {
    match resolve_group_forward_proxy_scope_for_provisioning(
        state,
        binding,
        assignments,
        existing_match,
        consumed_proxy_keys,
    )
    .await
    {
        Ok(scope) => Ok(scope),
        Err(err)
            if binding.node_shunt_enabled
                && is_group_node_shunt_unassigned_message(&err.to_string()) =>
        {
            let has_selectable_bound_proxy = {
                let manager = state.forward_proxy.lock().await;
                manager.has_selectable_bound_proxy_keys(&binding.bound_proxy_keys)
            };
            if has_selectable_bound_proxy {
                required_account_forward_proxy_scope(
                    Some(&binding.group_name),
                    binding.bound_proxy_keys.clone(),
                )
            } else {
                Err(err)
            }
        }
        Err(err) => Err(err),
    }
}

pub(crate) fn build_imported_oauth_pending_response(
    items: &[ImportOauthCredentialFileRequest],
) -> ImportedOauthValidationResponse {
    ImportedOauthValidationResponse {
        input_files: items.len(),
        unique_in_input: items.len(),
        duplicate_in_input: 0,
        rows: items
            .iter()
            .map(|item| ImportedOauthValidationRow {
                source_id: item.source_id.clone(),
                file_name: item.file_name.clone(),
                email: None,
                chatgpt_account_id: None,
                chatgpt_user_id: None,
                display_name: None,
                token_expires_at: None,
                matched_account: None,
                status: "pending".to_string(),
                detail: None,
                attempts: 0,
            })
            .collect(),
    }
}

pub(crate) fn build_imported_oauth_validation_response_from_rows(
    input_files: usize,
    rows: Vec<ImportedOauthValidationRow>,
) -> ImportedOauthValidationResponse {
    let duplicate_in_input = rows
        .iter()
        .filter(|row| row.status == IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT)
        .count();
    ImportedOauthValidationResponse {
        input_files,
        unique_in_input: rows.len().saturating_sub(duplicate_in_input),
        duplicate_in_input,
        rows,
    }
}

struct ImportedOauthImportContext<'a> {
    state: &'a Arc<AppState>,
    crypto_key: &'a [u8; 32],
    selected_source_ids: &'a HashSet<String>,
    cached_validation_results: &'a HashMap<String, ImportedOauthValidatedImportData>,
    resolved_group_binding: &'a ResolvedRequiredGroupProxyBinding,
    create_plan: &'a ImportedOauthCreatePlan,
    assignments: &'a UpstreamAccountNodeShuntAssignments,
    refresh_scope: &'a ForwardProxyRouteScope,
    batch: &'a mut ImportBatch,
    seen_keys: &'a mut HashSet<String>,
    consumed_proxy_keys: &'a mut HashSet<String>,
}

async fn run_imported_oauth_import(
    state: &Arc<AppState>,
    prepared: PreparedImportedOauthImport,
) -> Result<ImportedOauthImportResponse, (StatusCode, String)> {
    let PreparedImportedOauthImport {
        crypto_key,
        items,
        selected_source_ids,
        cached_validation_results,
        resolved_group_binding,
        create_plan,
        assignments,
        refresh_scope,
    } = prepared;
    let mut batch = ImportBatch::new(items.len(), selected_source_ids.len());
    let mut seen_keys = HashSet::new();
    let mut consumed_proxy_keys = HashSet::new();
    let mut context = ImportedOauthImportContext {
        state,
        crypto_key: &crypto_key,
        selected_source_ids: &selected_source_ids,
        cached_validation_results: &cached_validation_results,
        resolved_group_binding: &resolved_group_binding,
        create_plan: &create_plan,
        assignments: &assignments,
        refresh_scope: &refresh_scope,
        batch: &mut batch,
        seen_keys: &mut seen_keys,
        consumed_proxy_keys: &mut consumed_proxy_keys,
    };
    for item in items {
        process_imported_oauth_import_item(&mut context, item).await?;
    }
    drop(context);
    Ok(batch.into_response())
}

async fn process_imported_oauth_import_item(
    context: &mut ImportedOauthImportContext<'_>,
    item: ImportOauthCredentialFileRequest,
) -> Result<(), (StatusCode, String)> {
    if !context.selected_source_ids.contains(&item.source_id) {
        return Ok(());
    }
    let cached_validation = context
        .cached_validation_results
        .get(&item.source_id)
        .cloned();
    let normalized = match normalize_imported_oauth_for_import(cached_validation.as_ref(), &item) {
        Ok(value) => value,
        Err(message) => {
            context
                .batch
                .record_failure(invalid_imported_oauth_import_result(item, message));
            return Ok(());
        }
    };
    let match_key = imported_match_key(
        normalized.chatgpt_user_id.as_deref(),
        &normalized.email,
        &normalized.chatgpt_account_id,
    );
    if !context.seen_keys.insert(match_key) {
        context
            .batch
            .record_failure(failed_imported_oauth_import_result(
                normalized,
                None,
                None,
                "duplicate credential in selected import set".to_string(),
            ));
        return Ok(());
    }
    let existing_match = match load_imported_oauth_import_match(context, &normalized).await {
        Ok(value) => value,
        Err(()) => return Ok(()),
    };
    let matched_account = existing_match.as_ref().map(import_match_summary_from_row);
    let usage_scope = match resolve_imported_oauth_import_scope_or_record_failure(
        context,
        &normalized,
        matched_account.clone(),
        existing_match.as_ref(),
    )
    .await
    {
        Ok(scope) => scope,
        Err(()) => return Ok(()),
    };
    let probe = match load_imported_oauth_import_probe(
        context,
        cached_validation,
        &normalized,
        existing_match.as_ref(),
        &usage_scope,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            context
                .batch
                .record_failure(failed_imported_oauth_import_result(
                    normalized,
                    existing_match.as_ref().map(|row| row.id),
                    matched_account,
                    err,
                ));
            return Ok(());
        }
    };
    let (account_id, detail) = persist_imported_oauth_account(
        context.state.clone(),
        context.crypto_key,
        existing_match.as_ref(),
        &normalized,
        &probe,
        context.create_plan,
    )
    .await?;
    record_imported_oauth_success(
        context.batch,
        context.consumed_proxy_keys,
        &usage_scope,
        ImportedOauthSuccessRecord {
            existing: existing_match.is_some(),
            normalized,
            account_id,
            detail,
            matched_account,
        },
    );
    Ok(())
}

async fn load_imported_oauth_import_match(
    context: &mut ImportedOauthImportContext<'_>,
    normalized: &NormalizedImportedOauthCredentials,
) -> Result<Option<UpstreamAccountRow>, ()> {
    match load_imported_oauth_existing_match(context.state, normalized).await {
        Ok(value) => Ok(value),
        Err(err) => {
            context
                .batch
                .record_failure(failed_imported_oauth_import_result(
                    normalized.clone(),
                    None,
                    None,
                    err,
                ));
            Err(())
        }
    }
}

async fn resolve_imported_oauth_import_scope_or_record_failure(
    context: &mut ImportedOauthImportContext<'_>,
    normalized: &NormalizedImportedOauthCredentials,
    matched_account: Option<ImportedOauthMatchSummary>,
    existing_match: Option<&UpstreamAccountRow>,
) -> Result<ForwardProxyRouteScope, ()> {
    match resolve_imported_oauth_import_scope(
        context.state,
        context.resolved_group_binding,
        context.assignments,
        existing_match,
        context.consumed_proxy_keys,
    )
    .await
    {
        Ok(scope) => Ok(scope),
        Err(err) => {
            context
                .batch
                .record_failure(failed_imported_oauth_import_result(
                    normalized.clone(),
                    existing_match.map(|row| row.id),
                    matched_account,
                    err,
                ));
            Err(())
        }
    }
}

async fn load_imported_oauth_import_probe(
    context: &ImportedOauthImportContext<'_>,
    cached_validation: Option<ImportedOauthValidatedImportData>,
    normalized: &NormalizedImportedOauthCredentials,
    existing_match: Option<&UpstreamAccountRow>,
    usage_scope: &ForwardProxyRouteScope,
) -> Result<ImportedOauthProbeOutcome, String> {
    match cached_validation {
        Some(cached) => Ok(cached.probe),
        None => probe_imported_oauth_for_import(
            context.state,
            normalized,
            existing_match.map(|row| row.id),
            context.refresh_scope,
            usage_scope,
        )
        .await
        .map_err(|err| err.to_string()),
    }
}
