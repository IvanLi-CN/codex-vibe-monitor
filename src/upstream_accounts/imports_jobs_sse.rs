async fn build_imported_oauth_validation_response(
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
                rows.push(ImportedOauthValidationRow {
                    source_id: item.source_id.clone(),
                    file_name: item.file_name.clone(),
                    email: None,
                    chatgpt_account_id: None,
                    display_name: None,
                    token_expires_at: None,
                    matched_account: None,
                    status: IMPORT_VALIDATION_STATUS_INVALID.to_string(),
                    detail: Some(message),
                    attempts: 0,
                });
                continue;
            }
        };

        let match_key = imported_match_key(&normalized.email, &normalized.chatgpt_account_id);
        if !seen_keys.insert(match_key) {
            rows.push(ImportedOauthValidationRow {
                source_id: normalized.source_id,
                file_name: normalized.file_name,
                email: Some(normalized.email),
                chatgpt_account_id: Some(normalized.chatgpt_account_id),
                display_name: Some(normalized.display_name),
                token_expires_at: Some(normalized.token_expires_at),
                matched_account: None,
                status: IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT.to_string(),
                detail: Some("duplicate credential in current import selection".to_string()),
                attempts: 0,
            });
            continue;
        }
        let existing_match = match find_existing_import_match(
            &state.pool,
            &normalized.chatgpt_account_id,
            &normalized.email,
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                rows.push(ImportedOauthValidationRow {
                    source_id: normalized.source_id,
                    file_name: normalized.file_name,
                    email: Some(normalized.email),
                    chatgpt_account_id: Some(normalized.chatgpt_account_id),
                    display_name: Some(normalized.display_name),
                    token_expires_at: Some(normalized.token_expires_at),
                    matched_account: None,
                    status: IMPORT_VALIDATION_STATUS_ERROR.to_string(),
                    detail: Some(err.to_string()),
                    attempts: 0,
                });
                continue;
            }
        };
        let matched_account = existing_match.as_ref().map(import_match_summary_from_row);
        let usage_scope = match resolve_group_forward_proxy_scope_for_provisioning(
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
                rows.push(ImportedOauthValidationRow {
                    source_id: normalized.source_id,
                    file_name: normalized.file_name,
                    email: Some(normalized.email),
                    chatgpt_account_id: Some(normalized.chatgpt_account_id),
                    display_name: Some(normalized.display_name),
                    token_expires_at: Some(normalized.token_expires_at),
                    matched_account: matched_account.clone(),
                    status: IMPORT_VALIDATION_STATUS_ERROR.to_string(),
                    detail: Some(err.to_string()),
                    attempts: 0,
                });
                continue;
            }
        };
        let reservation_key = reserve_imported_oauth_node_shunt_scope(
            state,
            &normalized.source_id,
            existing_match.as_ref().map(|row| row.id),
            &usage_scope,
        )?;
        let (row, validated_import) = build_imported_oauth_validation_result(
            state,
            normalized,
            matched_account,
            &refresh_scope,
            &usage_scope,
        )
        .await;
        release_imported_oauth_node_shunt_scope(state, reservation_key);
        if let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = &usage_scope {
            if validated_import.is_some() {
                consumed_proxy_keys.insert(proxy_key.clone());
            }
        }
        rows.push(row);
    }

    Ok(build_imported_oauth_validation_response_from_rows(
        items.len(),
        rows,
    ))
}

fn build_imported_oauth_pending_response(
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

fn build_imported_oauth_validation_response_from_rows(
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

fn compute_imported_oauth_validation_counts(
    rows: &[ImportedOauthValidationRow],
) -> ImportedOauthValidationCounts {
    let mut counts = ImportedOauthValidationCounts::default();
    for row in rows {
        match row.status.as_str() {
            IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT => counts.duplicate_in_input += 1,
            IMPORT_VALIDATION_STATUS_OK => counts.ok += 1,
            IMPORT_VALIDATION_STATUS_OK_EXHAUSTED => counts.ok_exhausted += 1,
            IMPORT_VALIDATION_STATUS_INVALID => counts.invalid += 1,
            IMPORT_VALIDATION_STATUS_ERROR => counts.error += 1,
            "pending" => counts.pending += 1,
            _ => counts.error += 1,
        }
    }
    counts.checked =
        counts.duplicate_in_input + counts.ok + counts.ok_exhausted + counts.invalid + counts.error;
    counts
}

fn build_imported_oauth_snapshot_event(
    snapshot: ImportedOauthValidationResponse,
) -> ImportedOauthValidationSnapshotEvent {
    let counts = compute_imported_oauth_validation_counts(&snapshot.rows);
    ImportedOauthValidationSnapshotEvent { snapshot, counts }
}

fn compute_bulk_upstream_account_sync_counts(
    rows: &[BulkUpstreamAccountSyncRow],
) -> BulkUpstreamAccountSyncCounts {
    let mut counts = BulkUpstreamAccountSyncCounts {
        total: rows.len(),
        completed: 0,
        succeeded: 0,
        failed: 0,
        skipped: 0,
    };
    for row in rows {
        match row.status.as_str() {
            BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED => {
                counts.succeeded += 1;
                counts.completed += 1;
            }
            BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED => {
                counts.failed += 1;
                counts.completed += 1;
            }
            BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED => {
                counts.skipped += 1;
                counts.completed += 1;
            }
            _ => {}
        }
    }
    counts
}

fn with_bulk_upstream_account_sync_snapshot_status(
    mut snapshot: BulkUpstreamAccountSyncSnapshot,
    status: &str,
) -> BulkUpstreamAccountSyncSnapshot {
    snapshot.status = status.to_string();
    snapshot
}

fn build_bulk_upstream_account_sync_snapshot_event(
    snapshot: BulkUpstreamAccountSyncSnapshot,
) -> BulkUpstreamAccountSyncSnapshotEvent {
    let counts = compute_bulk_upstream_account_sync_counts(&snapshot.rows);
    BulkUpstreamAccountSyncSnapshotEvent { snapshot, counts }
}

fn imported_oauth_sse_event<T: Serialize>(event_name: &str, payload: &T) -> Option<Event> {
    match Event::default().event(event_name).json_data(payload) {
        Ok(event) => Some(event),
        Err(err) => {
            warn!(
                ?err,
                event_name, "failed to serialize imported oauth validation event"
            );
            None
        }
    }
}

fn bulk_upstream_account_sync_sse_event<T: Serialize>(
    event_name: &str,
    payload: &T,
) -> Option<Event> {
    match Event::default().event(event_name).json_data(payload) {
        Ok(event) => Some(event),
        Err(err) => {
            warn!(
                ?err,
                event_name, "failed to serialize bulk upstream account sync event"
            );
            None
        }
    }
}

fn imported_oauth_terminal_event_to_sse(
    terminal: &ImportedOauthValidationTerminalEvent,
) -> Option<Event> {
    match terminal {
        ImportedOauthValidationTerminalEvent::Completed(payload) => {
            imported_oauth_sse_event("completed", payload)
        }
        ImportedOauthValidationTerminalEvent::Failed(payload) => {
            imported_oauth_sse_event("failed", payload)
        }
        ImportedOauthValidationTerminalEvent::Cancelled(payload) => {
            imported_oauth_sse_event("cancelled", payload)
        }
    }
}

fn bulk_upstream_account_sync_terminal_event_to_sse(
    terminal: &BulkUpstreamAccountSyncTerminalEvent,
) -> Option<Event> {
    match terminal {
        BulkUpstreamAccountSyncTerminalEvent::Completed(payload) => {
            bulk_upstream_account_sync_sse_event("completed", payload)
        }
        BulkUpstreamAccountSyncTerminalEvent::Failed(payload) => {
            bulk_upstream_account_sync_sse_event("failed", payload)
        }
        BulkUpstreamAccountSyncTerminalEvent::Cancelled(payload) => {
            bulk_upstream_account_sync_sse_event("cancelled", payload)
        }
    }
}

async fn build_imported_oauth_validation_result(
    state: &AppState,
    normalized: NormalizedImportedOauthCredentials,
    matched_account: Option<ImportedOauthMatchSummary>,
    refresh_scope: &ForwardProxyRouteScope,
    usage_scope: &ForwardProxyRouteScope,
) -> (
    ImportedOauthValidationRow,
    Option<ImportedOauthValidatedImportData>,
) {
    match probe_imported_oauth_credentials(state, &normalized, refresh_scope, usage_scope).await {
        Ok(outcome) => (
            ImportedOauthValidationRow {
                source_id: normalized.source_id.clone(),
                file_name: normalized.file_name.clone(),
                email: Some(normalized.email.clone()),
                chatgpt_account_id: Some(normalized.chatgpt_account_id.clone()),
                display_name: Some(normalized.display_name.clone()),
                token_expires_at: Some(outcome.token_expires_at.clone()),
                matched_account,
                status: if outcome.exhausted {
                    IMPORT_VALIDATION_STATUS_OK_EXHAUSTED.to_string()
                } else {
                    IMPORT_VALIDATION_STATUS_OK.to_string()
                },
                detail: if outcome.exhausted {
                    Some("usage snapshot indicates the account is currently exhausted".to_string())
                } else {
                    outcome.usage_snapshot_warning.clone()
                },
                attempts: 1,
            },
            Some(ImportedOauthValidatedImportData {
                normalized,
                probe: outcome,
            }),
        ),
        Err(err) => (
            ImportedOauthValidationRow {
                source_id: normalized.source_id,
                file_name: normalized.file_name,
                email: Some(normalized.email),
                chatgpt_account_id: Some(normalized.chatgpt_account_id),
                display_name: Some(normalized.display_name),
                token_expires_at: Some(normalized.token_expires_at),
                matched_account,
                status: if is_import_invalid_error_message(&err.to_string()) {
                    IMPORT_VALIDATION_STATUS_INVALID.to_string()
                } else {
                    IMPORT_VALIDATION_STATUS_ERROR.to_string()
                },
                detail: Some(err.to_string()),
                attempts: 1,
            },
            None,
        ),
    }
}

async fn update_imported_oauth_validation_job_row(
    job: &Arc<ImportedOauthValidationJob>,
    row_index: usize,
    row: ImportedOauthValidationRow,
    validated_import: Option<ImportedOauthValidatedImportData>,
) {
    let counts = {
        let mut snapshot = job.snapshot.lock().await;
        if let Some(target) = snapshot.rows.get_mut(row_index) {
            *target = row.clone();
        } else {
            return;
        }
        snapshot.duplicate_in_input = snapshot
            .rows
            .iter()
            .filter(|candidate| candidate.status == IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT)
            .count();
        snapshot.unique_in_input = snapshot
            .rows
            .len()
            .saturating_sub(snapshot.duplicate_in_input);
        compute_imported_oauth_validation_counts(&snapshot.rows)
    };
    let source_id = row.source_id.clone();
    let mut validated_imports = job.validated_imports.lock().await;
    if let Some(validated_import) = validated_import {
        validated_imports.insert(source_id, validated_import);
    } else {
        validated_imports.remove(&source_id);
    }
    let _ = job.broadcaster.send(ImportedOauthValidationJobEvent::Row(
        ImportedOauthValidationRowEvent { row, counts },
    ));
}

async fn set_imported_oauth_validation_job_terminal(
    job: &Arc<ImportedOauthValidationJob>,
    terminal: ImportedOauthValidationTerminalEvent,
) {
    {
        let mut guard = job.terminal_event.lock().await;
        if guard.is_some() {
            return;
        }
        *guard = Some(terminal.clone());
    }
    let _ = job.broadcaster.send(match terminal {
        ImportedOauthValidationTerminalEvent::Completed(payload) => {
            ImportedOauthValidationJobEvent::Completed(payload)
        }
        ImportedOauthValidationTerminalEvent::Failed(payload) => {
            ImportedOauthValidationJobEvent::Failed(payload)
        }
        ImportedOauthValidationTerminalEvent::Cancelled(payload) => {
            ImportedOauthValidationJobEvent::Cancelled(payload)
        }
    });
}

async fn finish_imported_oauth_validation_job_completed(job: &Arc<ImportedOauthValidationJob>) {
    let snapshot = { job.snapshot.lock().await.clone() };
    set_imported_oauth_validation_job_terminal(
        job,
        ImportedOauthValidationTerminalEvent::Completed(build_imported_oauth_snapshot_event(
            snapshot,
        )),
    )
    .await;
}

async fn finish_imported_oauth_validation_job_failed(
    job: &Arc<ImportedOauthValidationJob>,
    error: String,
) {
    let snapshot = { job.snapshot.lock().await.clone() };
    set_imported_oauth_validation_job_terminal(
        job,
        ImportedOauthValidationTerminalEvent::Failed(ImportedOauthValidationFailedEvent {
            counts: compute_imported_oauth_validation_counts(&snapshot.rows),
            snapshot,
            error,
        }),
    )
    .await;
}

async fn finish_imported_oauth_validation_job_cancelled(job: &Arc<ImportedOauthValidationJob>) {
    let snapshot = { job.snapshot.lock().await.clone() };
    set_imported_oauth_validation_job_terminal(
        job,
        ImportedOauthValidationTerminalEvent::Cancelled(build_imported_oauth_snapshot_event(
            snapshot,
        )),
    )
    .await;
}

async fn update_bulk_upstream_account_sync_job_row(
    job: &Arc<BulkUpstreamAccountSyncJob>,
    row: BulkUpstreamAccountSyncRow,
) {
    let counts = {
        let mut snapshot = job.snapshot.lock().await;
        if let Some(target) = snapshot
            .rows
            .iter_mut()
            .find(|candidate| candidate.account_id == row.account_id)
        {
            *target = row.clone();
        } else {
            return;
        }
        compute_bulk_upstream_account_sync_counts(&snapshot.rows)
    };
    let _ = job.broadcaster.send(BulkUpstreamAccountSyncJobEvent::Row(
        BulkUpstreamAccountSyncRowEvent { row, counts },
    ));
}

async fn set_bulk_upstream_account_sync_job_terminal(
    job: &Arc<BulkUpstreamAccountSyncJob>,
    terminal: BulkUpstreamAccountSyncTerminalEvent,
) {
    let next_status = match &terminal {
        BulkUpstreamAccountSyncTerminalEvent::Completed(_) => {
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED
        }
        BulkUpstreamAccountSyncTerminalEvent::Failed(_) => {
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED
        }
        BulkUpstreamAccountSyncTerminalEvent::Cancelled(_) => {
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED
        }
    };
    {
        let mut guard = job.terminal_event.lock().await;
        if guard.is_some() {
            return;
        }
        *guard = Some(terminal.clone());
    }
    {
        let mut snapshot = job.snapshot.lock().await;
        snapshot.status = next_status.to_string();
    }
    let _ = job.broadcaster.send(match terminal {
        BulkUpstreamAccountSyncTerminalEvent::Completed(payload) => {
            BulkUpstreamAccountSyncJobEvent::Completed(payload)
        }
        BulkUpstreamAccountSyncTerminalEvent::Failed(payload) => {
            BulkUpstreamAccountSyncJobEvent::Failed(payload)
        }
        BulkUpstreamAccountSyncTerminalEvent::Cancelled(payload) => {
            BulkUpstreamAccountSyncJobEvent::Cancelled(payload)
        }
    });
}

async fn finish_bulk_upstream_account_sync_job_completed(job: &Arc<BulkUpstreamAccountSyncJob>) {
    let snapshot = with_bulk_upstream_account_sync_snapshot_status(
        job.snapshot.lock().await.clone(),
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED,
    );
    set_bulk_upstream_account_sync_job_terminal(
        job,
        BulkUpstreamAccountSyncTerminalEvent::Completed(
            build_bulk_upstream_account_sync_snapshot_event(snapshot),
        ),
    )
    .await;
}

async fn finish_bulk_upstream_account_sync_job_failed(
    job: &Arc<BulkUpstreamAccountSyncJob>,
    error: String,
) {
    let snapshot = with_bulk_upstream_account_sync_snapshot_status(
        job.snapshot.lock().await.clone(),
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED,
    );
    set_bulk_upstream_account_sync_job_terminal(
        job,
        BulkUpstreamAccountSyncTerminalEvent::Failed(BulkUpstreamAccountSyncFailedEvent {
            counts: compute_bulk_upstream_account_sync_counts(&snapshot.rows),
            snapshot,
            error,
        }),
    )
    .await;
}

async fn finish_bulk_upstream_account_sync_job_cancelled(job: &Arc<BulkUpstreamAccountSyncJob>) {
    let snapshot = with_bulk_upstream_account_sync_snapshot_status(
        job.snapshot.lock().await.clone(),
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED,
    );
    set_bulk_upstream_account_sync_job_terminal(
        job,
        BulkUpstreamAccountSyncTerminalEvent::Cancelled(
            build_bulk_upstream_account_sync_snapshot_event(snapshot),
        ),
    )
    .await;
}

fn schedule_imported_oauth_validation_job_cleanup(
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(15 * 60)).await;
        let should_remove = match runtime.get_validation_job(&job_id).await {
            Some(job) => job.terminal_event.lock().await.is_some(),
            None => false,
        };
        if should_remove {
            runtime.remove_validation_job(&job_id).await;
        }
    });
}

fn schedule_bulk_upstream_account_sync_job_cleanup(
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(15 * 60)).await;
        let should_remove = match runtime.get_bulk_sync_job(&job_id).await {
            Some(job) => job.terminal_event.lock().await.is_some(),
            None => false,
        };
        if should_remove {
            runtime.remove_bulk_sync_job(&job_id).await;
        }
    });
}

fn spawn_imported_oauth_validation_job(
    state: Arc<AppState>,
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
    items: Vec<ImportOauthCredentialFileRequest>,
    binding: ResolvedRequiredGroupProxyBinding,
    job: Arc<ImportedOauthValidationJob>,
) {
    tokio::spawn(async move {
        let run_result: Result<(), String> = async {
            let mut seen_keys = HashSet::new();
            let mut consumed_proxy_keys = HashSet::new();
            let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
                .await
                .map_err(|err| err.to_string())?;
            let refresh_scope = required_account_forward_proxy_scope(
                Some(&binding.group_name),
                binding.bound_proxy_keys.clone(),
            )
            .map_err(|err| err.to_string())?;

            for (row_index, item) in items.iter().enumerate() {
                if job.cancel.is_cancelled() {
                    finish_imported_oauth_validation_job_cancelled(&job).await;
                    return Ok(());
                }

                let normalized = match normalize_imported_oauth_credentials(item) {
                    Ok(value) => value,
                    Err(message) => {
                        update_imported_oauth_validation_job_row(
                            &job,
                            row_index,
                            ImportedOauthValidationRow {
                                source_id: item.source_id.clone(),
                                file_name: item.file_name.clone(),
                                email: None,
                                chatgpt_account_id: None,
                                display_name: None,
                                token_expires_at: None,
                                matched_account: None,
                                status: IMPORT_VALIDATION_STATUS_INVALID.to_string(),
                                detail: Some(message),
                                attempts: 0,
                            },
                            None,
                        )
                        .await;
                        continue;
                    }
                };

                let match_key =
                    imported_match_key(&normalized.email, &normalized.chatgpt_account_id);
                if !seen_keys.insert(match_key) {
                    update_imported_oauth_validation_job_row(
                        &job,
                        row_index,
                        ImportedOauthValidationRow {
                            source_id: normalized.source_id,
                            file_name: normalized.file_name,
                            email: Some(normalized.email),
                            chatgpt_account_id: Some(normalized.chatgpt_account_id),
                            display_name: Some(normalized.display_name),
                            token_expires_at: Some(normalized.token_expires_at),
                            matched_account: None,
                            status: IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT.to_string(),
                            detail: Some(
                                "duplicate credential in current import selection".to_string(),
                            ),
                            attempts: 0,
                        },
                        None,
                    )
                    .await;
                    continue;
                }
                let existing_match = match find_existing_import_match(
                    &state.pool,
                    &normalized.chatgpt_account_id,
                    &normalized.email,
                )
                .await
                {
                    Ok(value) => value,
                    Err(err) => {
                        update_imported_oauth_validation_job_row(
                            &job,
                            row_index,
                            ImportedOauthValidationRow {
                                source_id: normalized.source_id,
                                file_name: normalized.file_name,
                                email: Some(normalized.email),
                                chatgpt_account_id: Some(normalized.chatgpt_account_id),
                                display_name: Some(normalized.display_name),
                                token_expires_at: Some(normalized.token_expires_at),
                                matched_account: None,
                                status: IMPORT_VALIDATION_STATUS_ERROR.to_string(),
                                detail: Some(err.to_string()),
                                attempts: 0,
                            },
                            None,
                        )
                        .await;
                        continue;
                    }
                };
                let matched_account = existing_match.as_ref().map(import_match_summary_from_row);

                let usage_scope = match resolve_group_forward_proxy_scope_for_provisioning(
                    state.as_ref(),
                    &binding,
                    Some(&assignments),
                    existing_match.as_ref(),
                    &consumed_proxy_keys,
                )
                .await
                {
                    Ok(scope) => scope,
                    Err(err) => {
                        update_imported_oauth_validation_job_row(
                            &job,
                            row_index,
                            ImportedOauthValidationRow {
                                source_id: normalized.source_id,
                                file_name: normalized.file_name,
                                email: Some(normalized.email),
                                chatgpt_account_id: Some(normalized.chatgpt_account_id),
                                display_name: Some(normalized.display_name),
                                token_expires_at: Some(normalized.token_expires_at),
                                matched_account: matched_account.clone(),
                                status: IMPORT_VALIDATION_STATUS_ERROR.to_string(),
                                detail: Some(err.to_string()),
                                attempts: 0,
                            },
                            None,
                        )
                        .await;
                        continue;
                    }
                };
                let reservation_key = reserve_imported_oauth_node_shunt_scope(
                    state.as_ref(),
                    &normalized.source_id,
                    existing_match.as_ref().map(|row| row.id),
                    &usage_scope,
                )
                .map_err(|err| err.to_string())?;
                let (row, validated_import) = build_imported_oauth_validation_result(
                    state.as_ref(),
                    normalized,
                    matched_account,
                    &refresh_scope,
                    &usage_scope,
                )
                .await;
                release_imported_oauth_node_shunt_scope(state.as_ref(), reservation_key);
                if let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = &usage_scope {
                    if validated_import.is_some() {
                        consumed_proxy_keys.insert(proxy_key.clone());
                    }
                }
                update_imported_oauth_validation_job_row(&job, row_index, row, validated_import)
                    .await;
            }

            if job.cancel.is_cancelled() {
                finish_imported_oauth_validation_job_cancelled(&job).await;
                return Ok(());
            }

            finish_imported_oauth_validation_job_completed(&job).await;
            Ok(())
        }
        .await;

        if let Err(error) = run_result {
            finish_imported_oauth_validation_job_failed(&job, error).await;
        }

        schedule_imported_oauth_validation_job_cleanup(runtime, job_id);
    });
}

fn spawn_bulk_upstream_account_sync_job(
    state: Arc<AppState>,
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
    account_ids: Vec<i64>,
    job: Arc<BulkUpstreamAccountSyncJob>,
) {
    tokio::spawn(async move {
        let run_result: Result<(), String> = async {
            for account_id in account_ids {
                if job.cancel.is_cancelled() {
                    finish_bulk_upstream_account_sync_job_cancelled(&job).await;
                    return Ok(());
                }

                let maybe_row = load_upstream_account_row(&state.pool, account_id)
                    .await
                    .map_err(|err| err.to_string())?;
                let Some(row) = maybe_row else {
                    update_bulk_upstream_account_sync_job_row(
                        &job,
                        BulkUpstreamAccountSyncRow {
                            account_id,
                            display_name: format!("Account {account_id}"),
                            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED.to_string(),
                            detail: Some("account not found".to_string()),
                        },
                    )
                    .await;
                    continue;
                };

                if row.enabled == 0 {
                    update_bulk_upstream_account_sync_job_row(
                        &job,
                        BulkUpstreamAccountSyncRow {
                            account_id,
                            display_name: row.display_name.clone(),
                            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED.to_string(),
                            detail: Some("disabled accounts cannot be synced".to_string()),
                        },
                    )
                    .await;
                    continue;
                }

                let sync_result = state
                    .upstream_accounts
                    .account_ops
                    .run_manual_sync(state.clone(), account_id)
                    .await;
                let (status, detail) = match sync_result {
                    Ok(detail)
                        if detail.summary.display_status == UPSTREAM_ACCOUNT_STATUS_ACTIVE =>
                    {
                        (BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED, None)
                    }
                    Ok(detail) => (
                        BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED,
                        detail.summary.last_error.clone().or_else(|| {
                            Some(format!(
                                "sync finished with status {}",
                                detail.summary.display_status
                            ))
                        }),
                    ),
                    Err(err) => (
                        BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED,
                        Some(err.to_string()),
                    ),
                };
                update_bulk_upstream_account_sync_job_row(
                    &job,
                    BulkUpstreamAccountSyncRow {
                        account_id,
                        display_name: row.display_name,
                        status: status.to_string(),
                        detail,
                    },
                )
                .await;
            }

            finish_bulk_upstream_account_sync_job_completed(&job).await;
            Ok(())
        }
        .await;

        if let Err(err) = run_result {
            finish_bulk_upstream_account_sync_job_failed(&job, err).await;
        }

        schedule_bulk_upstream_account_sync_job_cleanup(runtime, job_id);
    });
}

async fn build_bulk_upstream_account_sync_job_response(
    job_id: String,
    job: &Arc<BulkUpstreamAccountSyncJob>,
) -> BulkUpstreamAccountSyncJobResponse {
    let snapshot = { job.snapshot.lock().await.clone() };
    BulkUpstreamAccountSyncJobResponse {
        job_id,
        counts: compute_bulk_upstream_account_sync_counts(&snapshot.rows),
        snapshot,
    }
}

pub(crate) async fn create_imported_oauth_validation_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ValidateImportedOauthAccountsRequest>,
) -> Result<Json<ImportedOauthValidationJobResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        payload.group_name.clone(),
        payload.group_bound_proxy_keys.clone(),
        payload.group_node_shunt_enabled,
    )
    .await?;
    let snapshot = build_imported_oauth_pending_response(&payload.items);
    let job_id = random_hex(16)?;
    let job = Arc::new(ImportedOauthValidationJob::new(snapshot.clone(), &binding));
    state
        .upstream_accounts
        .insert_validation_job(job_id.clone(), job.clone())
        .await;
    spawn_imported_oauth_validation_job(
        state.clone(),
        state.upstream_accounts.clone(),
        job_id.clone(),
        payload.items,
        binding,
        job,
    );
    Ok(Json(ImportedOauthValidationJobResponse {
        job_id,
        snapshot,
    }))
}

pub(crate) async fn create_bulk_upstream_account_sync_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<BulkUpstreamAccountSyncJobRequest>,
) -> Result<Json<BulkUpstreamAccountSyncJobResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let account_ids = normalize_bulk_upstream_account_ids(&payload.account_ids)?;
    let _creation_guard = state.upstream_accounts.bulk_sync_creation.lock().await;
    if let Some((job_id, job)) = state.upstream_accounts.get_running_bulk_sync_job().await {
        return Ok(Json(
            build_bulk_upstream_account_sync_job_response(job_id, &job).await,
        ));
    }
    let job_id = random_hex(16)?;
    let snapshot = BulkUpstreamAccountSyncSnapshot {
        job_id: job_id.clone(),
        status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
        rows: build_bulk_upstream_account_sync_pending_rows(&state.pool, &account_ids)
            .await
            .map_err(internal_error_tuple)?,
    };
    let counts = compute_bulk_upstream_account_sync_counts(&snapshot.rows);
    let job = Arc::new(BulkUpstreamAccountSyncJob::new(snapshot.clone()));
    state
        .upstream_accounts
        .insert_bulk_sync_job(job_id.clone(), job.clone())
        .await;
    drop(_creation_guard);
    spawn_bulk_upstream_account_sync_job(
        state.clone(),
        state.upstream_accounts.clone(),
        job_id.clone(),
        account_ids,
        job,
    );
    Ok(Json(BulkUpstreamAccountSyncJobResponse {
        job_id,
        snapshot,
        counts,
    }))
}

pub(crate) async fn stream_imported_oauth_validation_job_events(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    let job = state
        .upstream_accounts
        .get_validation_job(&job_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "validation job not found".to_string(),
            )
        })?;
    let snapshot = { job.snapshot.lock().await.clone() };
    let terminal = { job.terminal_event.lock().await.clone() };
    let initial_events = {
        let mut events = Vec::new();
        if let Some(event) =
            imported_oauth_sse_event("snapshot", &build_imported_oauth_snapshot_event(snapshot))
        {
            events.push(Ok(event));
        }
        if let Some(terminal_event) = terminal
            .as_ref()
            .and_then(imported_oauth_terminal_event_to_sse)
        {
            events.push(Ok(terminal_event));
        }
        stream::iter(events)
    };
    let job_id_for_updates = job_id.clone();
    let updates = BroadcastStream::new(job.broadcaster.subscribe()).filter_map(move |message| {
        let lagged_job_id = job_id_for_updates.clone();
        async move {
            match message {
                Ok(ImportedOauthValidationJobEvent::Row(payload)) => {
                    imported_oauth_sse_event("row", &payload).map(Ok)
                }
                Ok(ImportedOauthValidationJobEvent::Completed(payload)) => {
                    imported_oauth_sse_event("completed", &payload).map(Ok)
                }
                Ok(ImportedOauthValidationJobEvent::Failed(payload)) => {
                    imported_oauth_sse_event("failed", &payload).map(Ok)
                }
                Ok(ImportedOauthValidationJobEvent::Cancelled(payload)) => {
                    imported_oauth_sse_event("cancelled", &payload).map(Ok)
                }
                Err(err) => {
                    warn!(
                        ?err,
                        job_id = lagged_job_id,
                        "imported oauth validation sse lagging"
                    );
                    None
                }
            }
        }
    });

    Ok(Sse::new(initial_events.chain(updates))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

pub(crate) async fn get_bulk_upstream_account_sync_job(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<BulkUpstreamAccountSyncJobResponse>, (StatusCode, String)> {
    let job = state
        .upstream_accounts
        .get_bulk_sync_job(&job_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "bulk sync job not found".to_string()))?;
    Ok(Json(
        build_bulk_upstream_account_sync_job_response(job_id, &job).await,
    ))
}

pub(crate) async fn stream_bulk_upstream_account_sync_job_events(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    let job = state
        .upstream_accounts
        .get_bulk_sync_job(&job_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "bulk sync job not found".to_string()))?;
    let snapshot = { job.snapshot.lock().await.clone() };
    let terminal = { job.terminal_event.lock().await.clone() };
    let initial_events = {
        let mut events = Vec::new();
        if let Some(event) = bulk_upstream_account_sync_sse_event(
            "snapshot",
            &build_bulk_upstream_account_sync_snapshot_event(snapshot),
        ) {
            events.push(Ok(event));
        }
        if let Some(terminal_event) = terminal
            .as_ref()
            .and_then(bulk_upstream_account_sync_terminal_event_to_sse)
        {
            events.push(Ok(terminal_event));
        }
        stream::iter(events)
    };
    let job_id_for_updates = job_id.clone();
    let updates = BroadcastStream::new(job.broadcaster.subscribe()).filter_map(move |message| {
        let lagged_job_id = job_id_for_updates.clone();
        async move {
            match message {
                Ok(BulkUpstreamAccountSyncJobEvent::Row(payload)) => {
                    bulk_upstream_account_sync_sse_event("row", &payload).map(Ok)
                }
                Ok(BulkUpstreamAccountSyncJobEvent::Completed(payload)) => {
                    bulk_upstream_account_sync_sse_event("completed", &payload).map(Ok)
                }
                Ok(BulkUpstreamAccountSyncJobEvent::Failed(payload)) => {
                    bulk_upstream_account_sync_sse_event("failed", &payload).map(Ok)
                }
                Ok(BulkUpstreamAccountSyncJobEvent::Cancelled(payload)) => {
                    bulk_upstream_account_sync_sse_event("cancelled", &payload).map(Ok)
                }
                Err(err) => {
                    warn!(
                        ?err,
                        job_id = lagged_job_id,
                        "bulk upstream account sync sse lagging"
                    );
                    None
                }
            }
        }
    });

    Ok(Sse::new(initial_events.chain(updates))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

pub(crate) async fn cancel_imported_oauth_validation_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(job_id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let Some(job) = state.upstream_accounts.get_validation_job(&job_id).await else {
        return Err((
            StatusCode::NOT_FOUND,
            "validation job not found".to_string(),
        ));
    };

    if job.terminal_event.lock().await.is_some() {
        state.upstream_accounts.remove_validation_job(&job_id).await;
        return Ok(StatusCode::NO_CONTENT);
    }

    job.cancel.cancel();
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn cancel_bulk_upstream_account_sync_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(job_id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let Some(job) = state.upstream_accounts.get_bulk_sync_job(&job_id).await else {
        return Err((StatusCode::NOT_FOUND, "bulk sync job not found".to_string()));
    };

    if job.terminal_event.lock().await.is_some() {
        state.upstream_accounts.remove_bulk_sync_job(&job_id).await;
        return Ok(StatusCode::NO_CONTENT);
    }

    job.cancel.cancel();
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn bulk_update_upstream_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<BulkUpstreamAccountActionRequest>,
) -> Result<Json<BulkUpstreamAccountActionResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let action = normalize_bulk_upstream_account_action(&payload.action)?;
    let account_ids = normalize_bulk_upstream_account_ids(&payload.account_ids)?;
    let normalized_tag_ids = if matches!(
        action.as_str(),
        BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS | BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS
    ) {
        let tag_ids = validate_tag_ids(&state.pool, &payload.tag_ids).await?;
        if tag_ids.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "tagIds must contain at least one tag".to_string(),
            ));
        }
        tag_ids
    } else {
        Vec::new()
    };

    let mut results = Vec::with_capacity(account_ids.len());
    for account_id in &account_ids {
        let display_name = load_upstream_account_row(&state.pool, *account_id)
            .await
            .map_err(internal_error_tuple)?
            .map(|row| row.display_name);
        let outcome = apply_bulk_upstream_account_action(
            state.clone(),
            *account_id,
            action.as_str(),
            payload.group_name.clone(),
            normalized_tag_ids.clone(),
        )
        .await;
        let (status, detail) = match outcome {
            Ok(()) => ("succeeded".to_string(), None),
            Err((_, message)) => ("failed".to_string(), Some(message)),
        };
        results.push(BulkUpstreamAccountActionResult {
            account_id: *account_id,
            display_name,
            status,
            detail,
        });
    }

    let succeeded_count = results
        .iter()
        .filter(|result| result.status == "succeeded")
        .count();
    Ok(Json(BulkUpstreamAccountActionResponse {
        action,
        requested_count: account_ids.len(),
        completed_count: results.len(),
        succeeded_count,
        failed_count: results.len().saturating_sub(succeeded_count),
        results,
    }))
}

pub(crate) async fn validate_imported_oauth_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ValidateImportedOauthAccountsRequest>,
) -> Result<Json<ImportedOauthValidationResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        payload.group_name,
        payload.group_bound_proxy_keys,
        payload.group_node_shunt_enabled,
    )
    .await?;
    Ok(Json(
        build_imported_oauth_validation_response(state.as_ref(), &payload.items, &binding)
            .await
            .map_err(internal_error_tuple)?,
    ))
}

pub(crate) async fn import_validated_oauth_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ImportValidatedOauthAccountsRequest>,
) -> Result<Json<ImportedOauthImportResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let ImportValidatedOauthAccountsRequest {
        items,
        selected_source_ids,
        validation_job_id,
        group_name,
        group_bound_proxy_keys,
        group_node_shunt_enabled,
        group_note,
        concurrency_limit,
        tag_ids,
    } = payload;
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
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
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        group_note.clone(),
        group_note.is_some(),
        group_bound_proxy_keys.clone(),
        group_bound_proxy_keys.is_some(),
        group_concurrency_limit,
        concurrency_limit.is_some(),
        group_node_shunt_enabled,
        group_node_shunt_enabled.is_some(),
    );
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        group_name.clone(),
        group_bound_proxy_keys.clone(),
        group_node_shunt_enabled,
    )
    .await?;
    let group_name = Some(resolved_group_binding.group_name.clone());
    let tag_ids = validate_tag_ids(&state.pool, &tag_ids).await?;
    let cached_validation_results = if let Some(job_id) = normalize_optional_text(validation_job_id)
    {
        if let Some(job) = state.upstream_accounts.get_validation_job(&job_id).await {
            if job.target_group_name == resolved_group_binding.group_name
                && job.target_bound_proxy_keys == resolved_group_binding.bound_proxy_keys
                && job.target_node_shunt_enabled == resolved_group_binding.node_shunt_enabled
            {
                job.validated_imports.lock().await.clone()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };
    let input_files = items.len();
    let selected_files = selected_source_ids.len();
    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .map_err(internal_error_tuple)?;
    let refresh_scope = required_account_forward_proxy_scope(
        Some(&resolved_group_binding.group_name),
        resolved_group_binding.bound_proxy_keys.clone(),
    )
    .map_err(internal_error_tuple)?;

    let mut created = 0usize;
    let mut updated_existing = 0usize;
    let mut failed = 0usize;
    let mut seen_keys = HashSet::new();
    let mut consumed_proxy_keys = HashSet::new();
    let mut results = Vec::new();

    for item in items {
        if !selected_source_ids.contains(&item.source_id) {
            continue;
        }

        let cached_validation = cached_validation_results.get(&item.source_id).cloned();
        let normalized = match cached_validation.as_ref() {
            Some(cached) => cached.normalized.clone(),
            None => match normalize_imported_oauth_credentials(&item) {
                Ok(value) => value,
                Err(message) => {
                    failed += 1;
                    results.push(ImportedOauthImportResult {
                        source_id: item.source_id,
                        file_name: item.file_name,
                        email: None,
                        chatgpt_account_id: None,
                        account_id: None,
                        status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                        detail: Some(message),
                        matched_account: None,
                    });
                    continue;
                }
            },
        };

        let match_key = imported_match_key(&normalized.email, &normalized.chatgpt_account_id);
        if !seen_keys.insert(match_key) {
            failed += 1;
            results.push(ImportedOauthImportResult {
                source_id: normalized.source_id,
                file_name: normalized.file_name,
                email: Some(normalized.email),
                chatgpt_account_id: Some(normalized.chatgpt_account_id),
                account_id: None,
                status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                detail: Some("duplicate credential in selected import set".to_string()),
                matched_account: None,
            });
            continue;
        }

        let existing_match = match find_existing_import_match(
            &state.pool,
            &normalized.chatgpt_account_id,
            &normalized.email,
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                failed += 1;
                results.push(ImportedOauthImportResult {
                    source_id: normalized.source_id,
                    file_name: normalized.file_name,
                    email: Some(normalized.email),
                    chatgpt_account_id: Some(normalized.chatgpt_account_id),
                    account_id: None,
                    status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                    detail: Some(err.to_string()),
                    matched_account: None,
                });
                continue;
            }
        };
        let matched_account = existing_match.as_ref().map(import_match_summary_from_row);
        let usage_scope = match resolve_group_forward_proxy_scope_for_provisioning(
            state.as_ref(),
            &resolved_group_binding,
            Some(&assignments),
            existing_match.as_ref(),
            &consumed_proxy_keys,
        )
        .await
        {
            Ok(scope) => scope,
            Err(err) => {
                failed += 1;
                results.push(ImportedOauthImportResult {
                    source_id: normalized.source_id,
                    file_name: normalized.file_name,
                    email: Some(normalized.email),
                    chatgpt_account_id: Some(normalized.chatgpt_account_id),
                    account_id: existing_match.as_ref().map(|row| row.id),
                    status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                    detail: Some(err.to_string()),
                    matched_account,
                });
                continue;
            }
        };
        let probe = match cached_validation {
            Some(cached) => cached.probe,
            None => {
                let reservation_key = reserve_imported_oauth_node_shunt_scope(
                    state.as_ref(),
                    &normalized.source_id,
                    existing_match.as_ref().map(|row| row.id),
                    &usage_scope,
                )
                .map_err(internal_error_tuple)?;
                let probe_result = probe_imported_oauth_credentials(
                    state.as_ref(),
                    &normalized,
                    &refresh_scope,
                    &usage_scope,
                )
                .await;
                release_imported_oauth_node_shunt_scope(state.as_ref(), reservation_key);
                match probe_result {
                    Ok(value) => value,
                    Err(err) => {
                        failed += 1;
                        results.push(ImportedOauthImportResult {
                            source_id: normalized.source_id,
                            file_name: normalized.file_name,
                            email: Some(normalized.email),
                            chatgpt_account_id: Some(normalized.chatgpt_account_id),
                            account_id: existing_match.as_ref().map(|row| row.id),
                            status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                            detail: Some(err.to_string()),
                            matched_account,
                        });
                        continue;
                    }
                }
            }
        };

        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(probe.credentials.clone()),
        )
        .map_err(internal_error_tuple)?;
        let (persisted_account_id, import_warning) = if let Some(existing_row) =
            existing_match.as_ref()
        {
            let warning = state
                .upstream_accounts
                .account_ops
                .run_persist_imported_oauth(state.clone(), existing_row.id, probe.clone())
                .await?;
            (existing_row.id, warning)
        } else {
            let persisted_account_id = {
                let mut tx = state
                    .pool
                    .begin_with("BEGIN IMMEDIATE")
                    .await
                    .map_err(internal_error_tuple)?;
                ensure_display_name_available(&mut *tx, &normalized.display_name, None).await?;
                let account_id = upsert_oauth_account(
                    &mut tx,
                    OauthAccountUpsert {
                        account_id: None,
                        display_name: &normalized.display_name,
                        group_name: group_name.clone(),
                        is_mother: false,
                        note: None,
                        tag_ids: tag_ids.clone(),
                        requested_group_metadata_changes: requested_group_metadata_changes.clone(),
                        claims: &probe.claims,
                        encrypted_credentials,
                        token_expires_at: &probe.token_expires_at,
                    },
                )
                .await
                .map_err(internal_error_tuple)?;
                tx.commit().await.map_err(internal_error_tuple)?;
                account_id
            };

            let warning = state
                .upstream_accounts
                .account_ops
                .run_persist_imported_oauth(state.clone(), persisted_account_id, probe.clone())
                .await?;
            (persisted_account_id, warning)
        };

        if existing_match.is_some() {
            updated_existing += 1;
        } else {
            created += 1;
        }
        if let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = &usage_scope {
            consumed_proxy_keys.insert(proxy_key.clone());
        }
        results.push(ImportedOauthImportResult {
            source_id: normalized.source_id,
            file_name: normalized.file_name,
            email: Some(normalized.email),
            chatgpt_account_id: Some(normalized.chatgpt_account_id),
            account_id: Some(persisted_account_id),
            status: if existing_match.is_some() {
                IMPORT_RESULT_STATUS_UPDATED_EXISTING.to_string()
            } else {
                IMPORT_RESULT_STATUS_CREATED.to_string()
            },
            detail: import_warning,
            matched_account,
        });
    }

    Ok(Json(ImportedOauthImportResponse {
        summary: ImportedOauthImportSummary {
            input_files,
            selected_files,
            created,
            updated_existing,
            failed,
        },
        results,
    }))
}
