pub(crate) async fn find_existing_import_match(
    pool: &Pool<Sqlite>,
    chatgpt_user_id: Option<&str>,
    chatgpt_account_id: &str,
    email: &str,
) -> Result<Option<UpstreamAccountRow>> {
    if let Some(chatgpt_user_id) = chatgpt_user_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let user_id_matches = sqlx::query_as::<_, UpstreamAccountRow>(&format!(
            r#"
            SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS}
            FROM pool_upstream_accounts
            WHERE kind = ?1
              AND lower(trim(COALESCE(chatgpt_user_id, ''))) = lower(trim(?2))
            ORDER BY updated_at DESC, id DESC
            "#
        ))
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(chatgpt_user_id)
        .fetch_all(pool)
        .await?;
        if user_id_matches.len() > 1 {
            bail!(
                "multiple existing OAuth accounts match chatgpt_user_id {}",
                chatgpt_user_id
            );
        }
        if let Some(row) = user_id_matches.into_iter().next() {
            return Ok(Some(row));
        }
    }

    let email_matches = sqlx::query_as::<_, UpstreamAccountRow>(&format!(
        r#"
        SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS}
        FROM pool_upstream_accounts
        WHERE kind = ?1
          AND lower(trim(COALESCE(email, ''))) = lower(trim(?2))
        ORDER BY updated_at DESC, id DESC
        "#
    ))
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(email)
    .fetch_all(pool)
    .await?;
    if email_matches.len() > 1 {
        bail!("multiple existing OAuth accounts match email {}", email);
    }
    if let Some(row) = email_matches.into_iter().next() {
        return Ok(Some(row));
    }

    if chatgpt_user_id
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(None);
    }

    let account_id_matches = sqlx::query_as::<_, UpstreamAccountRow>(&format!(
        r#"
        SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS}
        FROM pool_upstream_accounts
        WHERE kind = ?1
          AND chatgpt_account_id = ?2
        ORDER BY updated_at DESC, id DESC
        "#
    ))
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(chatgpt_account_id)
    .fetch_all(pool)
    .await?;
    if account_id_matches.len() > 1 {
        bail!(
            "multiple existing OAuth accounts match account_id {}",
            chatgpt_account_id
        );
    }
    Ok(account_id_matches.into_iter().next())
}

async fn refresh_imported_oauth_probe_if_due(
    state: &AppState,
    imported: &NormalizedImportedOauthCredentials,
    refresh_scope: &ForwardProxyRouteScope,
    credentials: &mut StoredOauthCredentials,
    claims: &mut ChatgptJwtClaims,
    token_expires_at: &mut String,
) -> Result<()> {
    let refresh_due = parse_rfc3339_utc(token_expires_at)
        .map(|expires| {
            expires
                <= Utc::now()
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        })
        .unwrap_or(true);
    let Some(refresh_token) = refresh_due
        .then(|| oauth_refresh_token(credentials))
        .flatten()
    else {
        return Ok(());
    };
    let response =
        refresh_oauth_tokens_for_required_scope(state, refresh_scope, refresh_token).await?;
    let id_token_changed = response.id_token.is_some();
    *token_expires_at = apply_oauth_token_response(credentials, response);
    if id_token_changed {
        *claims = parse_chatgpt_jwt_claims(&credentials.id_token)?;
        claims.email = claims.email.take().or_else(|| Some(imported.email.clone()));
        claims.chatgpt_account_id = claims
            .chatgpt_account_id
            .take()
            .or_else(|| Some(imported.chatgpt_account_id.clone()));
    }
    Ok(())
}

pub(crate) async fn probe_imported_oauth_credentials(
    state: &AppState,
    imported: &NormalizedImportedOauthCredentials,
    refresh_scope: &ForwardProxyRouteScope,
    usage_scope: &ForwardProxyRouteScope,
) -> Result<ImportedOauthProbeOutcome, anyhow::Error> {
    let mut credentials = imported.credentials.clone();
    let mut claims = imported.claims.clone();
    let mut token_expires_at = imported.token_expires_at.clone();
    refresh_imported_oauth_probe_if_due(
        state,
        imported,
        refresh_scope,
        &mut credentials,
        &mut claims,
        &mut token_expires_at,
    )
    .await?;

    let usage_result = fetch_usage_snapshot_via_forward_proxy(
        state,
        usage_scope,
        &state.config,
        &credentials.access_token,
        claims
            .chatgpt_account_id
            .as_deref()
            .or(Some(imported.chatgpt_account_id.as_str())),
    )
    .await;
    let (snapshot, maintenance_proxy_snapshot, usage_snapshot_warning) = match usage_result {
        Ok((snapshot, proxy_snapshot)) => (Some(snapshot), Some(proxy_snapshot), None),
        Err(err) if is_import_invalid_error_message(&err.to_string()) => return Err(err),
        Err(err)
            if (err.to_string().contains("401") || err.to_string().contains("403"))
                && oauth_refresh_token(&credentials).is_some() =>
        {
            let refresh_token = oauth_refresh_token(&credentials).expect("checked refresh token");
            let response =
                refresh_oauth_tokens_for_required_scope(state, refresh_scope, refresh_token)
                    .await?;
            let id_token_changed = response.id_token.is_some();
            token_expires_at = apply_oauth_token_response(&mut credentials, response);
            if id_token_changed {
                claims = parse_chatgpt_jwt_claims(&credentials.id_token)?;
                claims.email = claims.email.or_else(|| Some(imported.email.clone()));
                claims.chatgpt_account_id = claims
                    .chatgpt_account_id
                    .or_else(|| Some(imported.chatgpt_account_id.clone()));
            }
            match fetch_usage_snapshot_via_forward_proxy(
                state,
                usage_scope,
                &state.config,
                &credentials.access_token,
                claims
                    .chatgpt_account_id
                    .as_deref()
                    .or(Some(imported.chatgpt_account_id.as_str())),
            )
            .await
            {
                Ok((snapshot, proxy_snapshot)) => (Some(snapshot), Some(proxy_snapshot), None),
                Err(retry_err)
                    if !is_import_invalid_error_message(&retry_err.to_string())
                        && !retry_err.to_string().contains("401")
                        && !retry_err.to_string().contains("403") =>
                {
                    (
                        None,
                        None,
                        Some(format!(
                            "usage snapshot unavailable during validation: {retry_err}"
                        )),
                    )
                }
                Err(retry_err) => return Err(retry_err),
            }
        }
        Err(err) => (
            None,
            None,
            Some(format!(
                "usage snapshot unavailable during validation: {err}"
            )),
        ),
    };

    Ok(ImportedOauthProbeOutcome {
        token_expires_at,
        credentials,
        claims,
        usage_snapshot: snapshot.clone(),
        maintenance_proxy_snapshot,
        exhausted: snapshot
            .as_ref()
            .is_some_and(imported_snapshot_is_exhausted),
        usage_snapshot_warning,
    })
}
