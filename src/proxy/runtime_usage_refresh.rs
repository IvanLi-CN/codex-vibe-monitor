use super::*;

#[derive(Debug, FromRow)]
pub(crate) struct PersistedInvocationIdentityRow {
    pub(crate) id: i64,
    pub(crate) status: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) source: String,
    pub(crate) model: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reported_cache_write_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) error_message: Option<String>,
    pub(crate) payload: Option<String>,
}

pub(crate) async fn load_persisted_invocation_identity_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Option<PersistedInvocationIdentityRow>> {
    sqlx::query_as::<_, PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status, failure_kind, source, model, input_tokens, output_tokens,
               cache_input_tokens, reported_cache_write_tokens, reasoning_tokens,
               total_tokens, error_message, payload
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(Into::into)
}

pub(crate) fn persisted_invocation_allows_proxy_record_update(
    existing_status: Option<&str>,
    existing_failure_kind: Option<&str>,
    incoming_status: &str,
) -> bool {
    invocation_status_is_in_flight(existing_status)
        || (!invocation_status_is_in_flight(Some(incoming_status))
            && invocation_status_is_recoverable_proxy_interrupted(
                existing_status,
                existing_failure_kind,
            ))
}

pub(crate) fn invocation_status_is_in_flight(status: Option<&str>) -> bool {
    matches!(
        status
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        INVOCATION_STATUS_RUNNING | INVOCATION_STATUS_PENDING
    )
}

pub(crate) fn invocation_status_is_recoverable_proxy_interrupted(
    status: Option<&str>,
    failure_kind: Option<&str>,
) -> bool {
    status
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case(INVOCATION_STATUS_INTERRUPTED)
        && failure_kind
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case(PROXY_FAILURE_INVOCATION_INTERRUPTED)
}
