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
