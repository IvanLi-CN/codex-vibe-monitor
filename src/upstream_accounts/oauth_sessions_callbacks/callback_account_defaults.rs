use super::*;

pub(crate) struct OauthCallbackAccountDefaults {
    pub(crate) display_name: String,
    pub(crate) chosen_email: Option<String>,
    pub(crate) group_name: Option<String>,
    pub(crate) is_mother: bool,
    pub(crate) note: Option<String>,
    pub(crate) tag_ids: Vec<i64>,
    pub(crate) group_metadata_changes: RequestedGroupMetadataChanges,
}

pub(crate) async fn resolve_oauth_callback_account_defaults(
    tx: &mut Transaction<'_, Sqlite>,
    session: &OauthLoginSessionRow,
    input: &PersistOauthCallbackInput,
) -> Result<OauthCallbackAccountDefaults, (StatusCode, String)> {
    let existing_account = if let Some(account_id) = session.account_id {
        let account = load_upstream_account_row_conn(tx.as_mut(), account_id)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
        if oauth_identity_requires_confirmation(&account, input) {
            mark_login_session_needs_identity_confirmation_with_executor(&mut **tx, session, input)
                .await
                .map_err(internal_error_tuple)?;
            return Err((
                StatusCode::CONFLICT,
                "OAuth identity confirmation required".to_string(),
            ));
        }
        Some(account)
    } else {
        let current_plan_type = normalize_plan_type(input.claims.chatgpt_plan_type.as_deref());
        if let Err((status, message)) = ensure_display_name_available_for_oauth_identity(
            &mut **tx,
            &input.display_name,
            None,
            input.claims.chatgpt_account_id.as_deref(),
            input.claims.chatgpt_user_id.as_deref(),
            session.group_name.as_deref(),
            current_plan_type.as_deref(),
        )
        .await
        {
            if status == StatusCode::CONFLICT {
                fail_login_session_with_executor(&mut **tx, &session.login_id, &message)
                    .await
                    .map_err(internal_error_tuple)?;
            }
            return Err((status, message));
        }
        None
    };

    Ok(if let Some(account) = existing_account {
        OauthCallbackAccountDefaults::for_existing_account(tx, account).await?
    } else {
        OauthCallbackAccountDefaults::for_new_account(session, input)
    })
}

impl OauthCallbackAccountDefaults {
    async fn for_existing_account(
        tx: &mut Transaction<'_, Sqlite>,
        account: UpstreamAccountRow,
    ) -> Result<Self, (StatusCode, String)> {
        Ok(Self {
            display_name: account.display_name,
            chosen_email: account.email,
            group_name: account.group_name,
            is_mother: account.is_mother != 0,
            note: account.note,
            tag_ids: current_account_tag_ids_with_executor(tx.as_mut(), account.id)
                .await
                .map_err(internal_error_tuple)?,
            group_metadata_changes: RequestedGroupMetadataChanges::default(),
        })
    }

    fn for_new_account(session: &OauthLoginSessionRow, input: &PersistOauthCallbackInput) -> Self {
        Self {
            display_name: input.display_name.clone(),
            chosen_email: input.chosen_email.clone(),
            group_name: session.group_name.clone(),
            is_mother: session.is_mother != 0,
            note: session.note.clone(),
            tag_ids: parse_tag_ids_json(session.tag_ids_json.as_deref()),
            group_metadata_changes: build_requested_group_metadata_changes(
                RequestedGroupMetadataInput::from_oauth_login_session(session),
            ),
        }
    }
}
