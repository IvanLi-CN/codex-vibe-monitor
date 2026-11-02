use super::*;
use sqlx::Transaction;

pub(crate) async fn persist_oauth_token_lifecycle_metadata(
    pool: &Pool<Sqlite>,
    account_id: i64,
    lifecycle_metadata: &OAuthTokenLifecycleMetadata,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET access_token_expiry_source = ?2,
            access_token_expiry_observed_at = ?3,
            refresh_token_expires_at = CASE WHEN ?7 = 1 THEN ?4 ELSE refresh_token_expires_at END,
            refresh_token_expiry_source = CASE WHEN ?7 = 1 THEN ?5 ELSE refresh_token_expiry_source END,
            refresh_token_expiry_state = CASE WHEN ?7 = 1 THEN ?6 ELSE refresh_token_expiry_state END,
            refresh_token_expiry_observed_at = CASE WHEN ?7 = 1 THEN ?3 ELSE refresh_token_expiry_observed_at END,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(lifecycle_metadata.access_token_source.as_deref())
    .bind(lifecycle_metadata.access_token_observed_at.as_deref())
    .bind(lifecycle_metadata.refresh_token_expires_at.as_deref())
    .bind(lifecycle_metadata.refresh_token_source.as_deref())
    .bind(lifecycle_metadata.refresh_token_state.as_deref())
    .bind(i64::from(lifecycle_metadata.refresh_token_metadata_update))
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn persist_oauth_credentials_with_executor(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    crypto_key: &[u8; 32],
    credentials: &StoredOauthCredentials,
    token_expires_at: &str,
    lifecycle_metadata: Option<&OAuthTokenLifecycleMetadata>,
) -> Result<()> {
    let claims = parse_chatgpt_jwt_claims(&credentials.id_token).unwrap_or_default();
    let Some(row) = load_upstream_account_row_conn(tx.as_mut(), account_id).await? else {
        bail!("account not found");
    };
    let effective_row_plan_type =
        resolve_effective_plan_type_for_account(tx.as_mut(), account_id, row.plan_type.as_deref())
            .await?;
    let next_verified_email = normalize_email_value(claims.email.clone());
    let should_follow_verified_email = should_replace_email_after_verified_email_change(
        row.email.as_deref(),
        row.verified_email.as_deref(),
    );
    let next_email = if should_follow_verified_email {
        next_verified_email.clone().or_else(|| row.email.clone())
    } else {
        row.email.clone()
    };
    let previous_display_email_source = if should_follow_verified_email {
        row.email.as_deref().or(row.verified_email.as_deref())
    } else {
        row.email.as_deref()
    };
    let next_display_name = resolve_display_name_after_email_change(
        Some(row.display_name.as_str()),
        None,
        previous_display_email_source,
        next_email.as_deref(),
    )
    .unwrap_or(row.display_name.clone());
    ensure_display_name_available_for_oauth_identity(
        tx.as_mut(),
        &next_display_name,
        Some(account_id),
        claims
            .chatgpt_account_id
            .as_deref()
            .or(row.chatgpt_account_id.as_deref()),
        claims
            .chatgpt_user_id
            .as_deref()
            .or(row.chatgpt_user_id.as_deref()),
        row.group_name.as_deref(),
        normalize_plan_type(claims.chatgpt_plan_type.as_deref())
            .or(effective_row_plan_type)
            .as_deref(),
    )
    .await
    .map_err(|(_, message)| anyhow!(message))?;
    let encrypted =
        encrypt_credentials(crypto_key, &StoredCredentials::Oauth(credentials.clone()))?;
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET encrypted_credentials = ?2,
            token_expires_at = ?3,
            has_refresh_token = ?11,
            access_token_expiry_source = COALESCE(?12, access_token_expiry_source),
            access_token_expiry_observed_at = COALESCE(?13, access_token_expiry_observed_at),
            refresh_token_expires_at = CASE WHEN ?16 = 1 THEN ?14 ELSE refresh_token_expires_at END,
            refresh_token_expiry_source = CASE WHEN ?16 = 1 THEN ?15 ELSE refresh_token_expiry_source END,
            refresh_token_expiry_state = CASE WHEN ?16 = 1 THEN ?17 ELSE refresh_token_expiry_state END,
            refresh_token_expiry_observed_at = CASE WHEN ?16 = 1 THEN ?18 ELSE refresh_token_expiry_observed_at END,
            last_refreshed_at = ?4,
            email = ?5,
            verified_email = ?6,
            display_name = ?7,
            chatgpt_account_id = COALESCE(?8, chatgpt_account_id),
            chatgpt_user_id = COALESCE(?9, chatgpt_user_id),
            plan_type = COALESCE(?10, plan_type),
            plan_type_observed_at = CASE
                WHEN NULLIF(TRIM(?10), '') IS NOT NULL THEN ?4
                ELSE plan_type_observed_at
            END,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(encrypted)
    .bind(token_expires_at)
    .bind(&now_iso)
    .bind(next_email)
    .bind(next_verified_email)
    .bind(next_display_name)
    .bind(claims.chatgpt_account_id)
    .bind(claims.chatgpt_user_id)
    .bind(claims.chatgpt_plan_type)
    .bind(if oauth_credentials_have_refresh_token(credentials) {
        1
    } else {
        0
    })
    .bind(lifecycle_metadata.and_then(|metadata| metadata.access_token_source.as_deref()))
    .bind(lifecycle_metadata.and_then(|metadata| metadata.access_token_observed_at.as_deref()))
    .bind(lifecycle_metadata.and_then(|metadata| metadata.refresh_token_expires_at.as_deref()))
    .bind(lifecycle_metadata.and_then(|metadata| metadata.refresh_token_source.as_deref()))
    .bind(i64::from(
        lifecycle_metadata.is_some_and(|metadata| metadata.refresh_token_metadata_update),
    ))
    .bind(lifecycle_metadata.and_then(|metadata| metadata.refresh_token_state.as_deref()))
    .bind(lifecycle_metadata.and_then(|metadata| metadata.refresh_token_observed_at.as_deref()))
    .execute(tx.as_mut())
    .await?;
    Ok(())
}

pub(crate) fn build_token_lifecycle(
    row: &UpstreamAccountRow,
    credential_refresh: &UpstreamAccountRefreshStatus,
    now: DateTime<Utc>,
) -> Option<UpstreamTokenLifecycle> {
    if row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        return None;
    }
    let refresh_attempt_is_newer = match (
        credential_refresh
            .last_attempt_at
            .as_deref()
            .and_then(parse_rfc3339_utc),
        credential_refresh
            .last_success_at
            .as_deref()
            .and_then(parse_rfc3339_utc),
    ) {
        (Some(last_attempt), Some(last_success)) => last_attempt > last_success,
        (Some(_), None) => credential_refresh.state != UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED,
        _ => false,
    };
    let access_expired = row
        .token_expires_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .is_some_and(|expires_at| expires_at <= now);
    let access_state = if access_expired {
        "expired"
    } else if refresh_attempt_is_newer && row.token_expires_at.is_some() {
        "stale"
    } else if row.token_expires_at.is_some() {
        "known"
    } else {
        "unconfirmed"
    };

    let refresh_present = row.has_refresh_token.unwrap_or_default() != 0;
    let refresh_expired = row
        .refresh_token_expires_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .is_some_and(|expires_at| expires_at <= now);
    let refresh_state = if !refresh_present {
        "missing"
    } else if row.refresh_token_expiry_state.as_deref() == Some("unparseable") {
        "unparseable"
    } else if refresh_expired {
        "expired"
    } else if refresh_attempt_is_newer && row.refresh_token_expires_at.is_some() {
        "stale"
    } else if row.refresh_token_expires_at.is_some() {
        "known"
    } else {
        "not_provided"
    };

    Some(UpstreamTokenLifecycle {
        access_token: UpstreamTokenLifecycleEntry {
            expires_at: row.token_expires_at.clone(),
            state: access_state.to_string(),
            source: row.access_token_expiry_source.clone().or_else(|| {
                row.token_expires_at
                    .as_ref()
                    .map(|_| "legacy_token_expires_at".to_string())
            }),
            observed_at: row
                .access_token_expiry_observed_at
                .clone()
                .or_else(|| row.last_refreshed_at.clone()),
        },
        refresh_token: UpstreamRefreshTokenLifecycleEntry {
            present: refresh_present,
            expires_at: row.refresh_token_expires_at.clone(),
            state: refresh_state.to_string(),
            source: row.refresh_token_expiry_source.clone().or_else(|| {
                (refresh_present && refresh_state == "not_provided")
                    .then_some("not_provided".to_string())
            }),
            observed_at: row
                .refresh_token_expiry_observed_at
                .clone()
                .or_else(|| row.last_refreshed_at.clone()),
        },
    })
}
