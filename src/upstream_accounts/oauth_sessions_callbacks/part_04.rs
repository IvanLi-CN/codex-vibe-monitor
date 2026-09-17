async fn current_account_tag_ids_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    account_id: i64,
) -> Result<Vec<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT tag_id
        FROM pool_upstream_account_tags
        WHERE account_id = ?1
        ORDER BY tag_id ASC
        "#,
    )
    .bind(account_id)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}

pub(crate) fn parse_manual_oauth_callback(
    callback_url: &str,
    expected_redirect_uri: &str,
) -> Result<OauthCallbackQuery> {
    let trimmed = callback_url.trim();
    if trimmed.is_empty() {
        bail!("Callback URL is required.");
    }

    let expected =
        Url::parse(expected_redirect_uri).context("failed to parse stored redirect URI")?;
    let parsed = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Url::parse(trimmed).context("callback URL must be a valid absolute URL")?
    } else if trimmed.starts_with('?') || trimmed.contains("code=") || trimmed.contains("state=") {
        let mut url = expected.clone();
        let query = trimmed.strip_prefix('?').unwrap_or(trimmed);
        url.set_query(Some(query));
        url
    } else {
        bail!("Callback URL must be a full URL or query string.");
    };

    if parsed.scheme() != expected.scheme()
        || parsed.host_str() != expected.host_str()
        || parsed.port_or_known_default() != expected.port_or_known_default()
        || parsed.path() != expected.path()
    {
        bail!("Callback URL does not match the generated localhost redirect address.");
    }

    let mut query = OauthCallbackQuery {
        code: None,
        state: None,
        error: None,
        error_description: None,
    };
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" if query.code.is_none() => query.code = Some(value.into_owned()),
            "state" if query.state.is_none() => query.state = Some(value.into_owned()),
            "error" if query.error.is_none() => query.error = Some(value.into_owned()),
            "error_description" if query.error_description.is_none() => {
                query.error_description = Some(value.into_owned())
            }
            _ => {}
        }
    }
    Ok(query)
}
