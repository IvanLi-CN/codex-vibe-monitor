use super::*;
use crate::api::{
    RuntimeStickyMutation, broadcast_prompt_cache_conversation_changed,
    broadcast_prompt_cache_conversation_sticky_route_changed,
    upsert_runtime_prompt_cache_conversation_sticky_route,
};

#[derive(Debug, Clone, Copy)]
pub(crate) enum UpstreamCapabilityAxis {
    ResponseEndpoint,
    ChatCompletionsEndpoint,
    ImageEndpoint,
    ResponseImageTool,
    CodexImagegen,
    StandaloneSearch,
}

struct PoolRouteSuccessOutcome {
    sticky_mutation: RuntimeStickyMutation,
    availability_increased: bool,
}

fn route_failure_precedes_request(
    stored_failure_at: &str,
    request_started_at_utc: DateTime<Utc>,
) -> bool {
    let Some(failure_at) = parse_to_utc_datetime(stored_failure_at) else {
        return false;
    };
    // Legacy rows only have second precision. When both values fall in the
    // same second, their ordering cannot be proven, so recovery must fail
    // closed instead of clearing a potentially newer failure.
    if !stored_failure_at.contains('.')
        && failure_at.timestamp() == request_started_at_utc.timestamp()
    {
        return false;
    }
    failure_at < request_started_at_utc
}

pub(crate) async fn pool_account_allows_model_route_availability_publish(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<bool> {
    let eligible = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM pool_upstream_accounts
            WHERE id = ?1
              AND enabled != 0
              AND deleted_at IS NULL
              AND status = ?2
              AND last_route_failure_at IS NULL
              AND last_route_failure_kind IS NULL
              AND cooldown_until IS NULL
              AND consecutive_route_failures = 0
              AND temporary_route_failure_streak_started_at IS NULL
        )
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .fetch_one(pool)
    .await?;
    Ok(eligible != 0)
}

async fn publish_pool_routing_availability_if_account_eligible(state: &AppState, account_id: i64) {
    match pool_account_allows_model_route_availability_publish(&state.pool, account_id).await {
        Ok(true) => publish_pool_routing_availability(state),
        Ok(false) => debug!(
            account_id,
            "suppressing pool availability publication because the recovered account is not selectable"
        ),
        Err(err) => warn!(
            account_id,
            error = %err,
            "failed to verify account eligibility before publishing pool availability"
        ),
    }
}

impl UpstreamCapabilityAxis {
    fn observed_column(self) -> &'static str {
        match self {
            Self::ResponseEndpoint => "response_endpoint_capability",
            Self::ChatCompletionsEndpoint => "chat_completions_capability",
            Self::ImageEndpoint => "image_endpoint_capability",
            Self::ResponseImageTool => "response_image_tool_capability",
            Self::CodexImagegen => "codex_imagegen_capability",
            Self::StandaloneSearch => "standalone_search_capability",
        }
    }

    fn observed_at_column(self) -> &'static str {
        match self {
            Self::ResponseEndpoint => "response_endpoint_capability_observed_at",
            Self::ChatCompletionsEndpoint => "chat_completions_capability_observed_at",
            Self::ImageEndpoint => "image_endpoint_capability_observed_at",
            Self::ResponseImageTool => "response_image_tool_capability_observed_at",
            Self::CodexImagegen => "codex_imagegen_capability_observed_at",
            Self::StandaloneSearch => "standalone_search_capability_observed_at",
        }
    }

    fn reason_column(self) -> &'static str {
        match self {
            Self::ResponseEndpoint => "response_endpoint_capability_reason",
            Self::ChatCompletionsEndpoint => "chat_completions_capability_reason",
            Self::ImageEndpoint => "image_endpoint_capability_reason",
            Self::ResponseImageTool => "response_image_tool_capability_reason",
            Self::CodexImagegen => "codex_imagegen_capability_reason",
            Self::StandaloneSearch => "standalone_search_capability_reason",
        }
    }

    fn success_reason(self) -> &'static str {
        match self {
            Self::ResponseEndpoint => "response endpoint request succeeded",
            Self::ChatCompletionsEndpoint => "chat completions endpoint request succeeded",
            Self::ImageEndpoint => "image endpoint request succeeded",
            Self::ResponseImageTool => "response image tool request succeeded",
            Self::CodexImagegen => "Codex imagegen namespace request succeeded",
            Self::StandaloneSearch => "standalone search endpoint request succeeded",
        }
    }
}

pub(crate) async fn record_pool_route_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    invoke_id: Option<&str>,
) -> Result<()> {
    record_pool_route_success_inner(
        pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        None,
        invoke_id,
        None,
        None,
    )
    .await
    .map(|_| ())
}

pub(crate) async fn record_pool_route_success_with_affinity_generation(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    invoke_id: Option<&str>,
    sticky_affinity_generation: Option<i64>,
) -> Result<()> {
    record_pool_route_success_with_affinity_generation_for_attempt(
        pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        prompt_cache_key,
        invoke_id,
        None,
        sticky_affinity_generation,
    )
    .await
    .map(|_| ())
}

pub(crate) async fn record_pool_route_success_with_affinity_generation_and_broadcast(
    state: &AppState,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
) -> Result<()> {
    let outcome = record_pool_route_success_inner(
        &state.pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        prompt_cache_key,
        invoke_id,
        attempt_id,
        sticky_affinity_generation,
    )
    .await?;
    if outcome.sticky_mutation.writes_conversation_operation()
        && let Some(prompt_cache_key) = prompt_cache_key.filter(|key| sticky_key == Some(*key))
    {
        broadcast_prompt_cache_conversation_changed(state, prompt_cache_key);
    }
    if let Some(previous_upstream_account_id) =
        outcome.sticky_mutation.previous_upstream_account_id()
        && let Some(sticky_key) = sticky_key
    {
        broadcast_prompt_cache_conversation_sticky_route_changed(
            state,
            sticky_key,
            previous_upstream_account_id,
            account_id,
        );
    }
    if outcome.availability_increased {
        publish_pool_routing_availability_if_account_eligible(state, account_id).await;
    }
    Ok(())
}

pub(crate) async fn record_pool_route_success_with_affinity_generation_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
) -> Result<()> {
    record_pool_route_success_inner(
        pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        prompt_cache_key,
        invoke_id,
        attempt_id,
        sticky_affinity_generation,
    )
    .await
    .map(|_| ())
}

async fn record_pool_route_success_inner(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
) -> Result<PoolRouteSuccessOutcome> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let now_iso = format_utc_iso(Utc::now());
    let sticky_now_iso = format_utc_iso_precise(Utc::now());
    let model_request_started_at_iso = format_naive_precise(
        request_started_at_utc
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    // Model recovery is independently fenced by the attempt timestamp, so it
    // must still run when a newer account-level failure makes the account
    // update stale.
    let model_route_recovered = if let Some(attempt_id) = attempt_id {
        record_model_route_success_from_attempt(
            pool,
            account_id,
            attempt_id,
            Some(&model_request_started_at_iso),
        )
        .await?
    } else {
        false
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let account = sqlx::query(
        r#"
        SELECT status, last_error, last_error_at, last_route_failure_at,
               last_route_failure_kind, cooldown_until, consecutive_route_failures,
               temporary_route_failure_streak_started_at
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(account) = account else {
        tx.commit().await?;
        return Ok(PoolRouteSuccessOutcome {
            sticky_mutation: RuntimeStickyMutation::Unchanged,
            availability_increased: false,
        });
    };
    let last_route_failure_at = account.try_get::<Option<String>, _>("last_route_failure_at")?;
    if last_route_failure_at.as_deref().is_some_and(|failure_at| {
        !route_failure_precedes_request(failure_at, request_started_at_utc)
    }) {
        tx.commit().await?;
        return Ok(PoolRouteSuccessOutcome {
            sticky_mutation: RuntimeStickyMutation::Unchanged,
            // A newer or ambiguous account failure keeps the account out of
            // routing even if model evidence independently recovered.
            availability_increased: false,
        });
    }
    let account_recovered = account.try_get::<String, _>("status")?
        != UPSTREAM_ACCOUNT_STATUS_ACTIVE
        || account
            .try_get::<Option<String>, _>("last_error")?
            .is_some()
        || account
            .try_get::<Option<String>, _>("last_error_at")?
            .is_some()
        || last_route_failure_at.is_some()
        || account
            .try_get::<Option<String>, _>("last_route_failure_kind")?
            .is_some()
        || account
            .try_get::<Option<String>, _>("cooldown_until")?
            .is_some()
        || account.try_get::<i64, _>("consecutive_route_failures")? != 0
        || account
            .try_get::<Option<String>, _>("temporary_route_failure_streak_started_at")?
            .is_some();
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_selected_at = COALESCE(last_selected_at, ?3),
            last_error = NULL,
            last_error_at = NULL,
            last_route_failure_at = NULL,
            last_route_failure_kind = NULL,
            cooldown_until = NULL,
            consecutive_route_failures = 0,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(&now_iso)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut sticky_mutation = RuntimeStickyMutation::Unchanged;
    if let Some(sticky_key) = sticky_key {
        sticky_mutation = upsert_runtime_prompt_cache_conversation_sticky_route(
            pool,
            sticky_key,
            prompt_cache_key,
            account_id,
            &sticky_now_iso,
            invoke_id,
            attempt_id,
            sticky_affinity_generation,
        )
        .await?;
        if sticky_mutation == RuntimeStickyMutation::Unchanged
            && prompt_cache_key.is_some_and(|key| key == sticky_key)
        {
            debug!(
                sticky_key,
                account_id,
                invoke_id,
                expected_generation = sticky_affinity_generation,
                "pool route success did not change the scoped sticky target"
            );
        }
    }
    record_upstream_account_action_for_attempt(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED,
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: None,
            reason_message: None,
            http_status: None,
            failure_kind: None,
            invoke_id,
            sticky_key,
            occurred_at: &now_iso,
        },
        attempt_id,
    )
    .await?;
    Ok(PoolRouteSuccessOutcome {
        sticky_mutation,
        availability_increased: model_route_recovered || account_recovered,
    })
}

pub(crate) async fn record_pool_route_success_with_image_intent(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    invoke_id: Option<&str>,
    image_intent: ImageIntent,
) -> Result<()> {
    record_pool_route_success_with_image_intent_for_attempt(
        pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        invoke_id,
        image_intent,
        None,
    )
    .await
}

pub(crate) async fn record_pool_route_success_for_endpoint_with_image_intent(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
) -> Result<()> {
    record_pool_route_success_for_endpoint_with_image_intent_for_attempt(
        pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        invoke_id,
        endpoint,
        image_intent,
        None,
    )
    .await
}

pub(crate) async fn record_pool_route_success_with_image_intent_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    invoke_id: Option<&str>,
    image_intent: ImageIntent,
    attempt_id: Option<i64>,
) -> Result<()> {
    record_pool_route_success_for_endpoint_with_image_intent_for_attempt(
        pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        invoke_id,
        "",
        image_intent,
        attempt_id,
    )
    .await
}

pub(crate) async fn record_pool_route_success_for_endpoint_with_image_intent_and_affinity_generation_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    codex_imagegen_rewrite: Option<&Value>,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
) -> Result<()> {
    let _ = record_pool_route_success_inner(
        pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        prompt_cache_key,
        invoke_id,
        attempt_id,
        sticky_affinity_generation,
    )
    .await?;
    record_pool_route_success_capability_observations(
        pool,
        account_id,
        endpoint,
        image_intent,
        codex_imagegen_rewrite,
    )
    .await
}

pub(crate) async fn record_pool_route_success_for_endpoint_with_image_intent_and_affinity_generation_for_attempt_and_broadcast(
    state: &AppState,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    codex_imagegen_rewrite: Option<&Value>,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
) -> Result<()> {
    let outcome = record_pool_route_success_inner(
        &state.pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        prompt_cache_key,
        invoke_id,
        attempt_id,
        sticky_affinity_generation,
    )
    .await?;
    if outcome.sticky_mutation.writes_conversation_operation()
        && let Some(prompt_cache_key) = prompt_cache_key.filter(|key| sticky_key == Some(*key))
    {
        broadcast_prompt_cache_conversation_changed(state, prompt_cache_key);
    }
    if let Some(previous_upstream_account_id) =
        outcome.sticky_mutation.previous_upstream_account_id()
        && let Some(sticky_key) = sticky_key
    {
        broadcast_prompt_cache_conversation_sticky_route_changed(
            state,
            sticky_key,
            previous_upstream_account_id,
            account_id,
        );
    }
    if outcome.availability_increased {
        publish_pool_routing_availability_if_account_eligible(state, account_id).await;
    }
    record_pool_route_success_capability_observations(
        &state.pool,
        account_id,
        endpoint,
        image_intent,
        codex_imagegen_rewrite,
    )
    .await
}

pub(crate) async fn record_pool_route_success_for_endpoint_with_image_intent_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    attempt_id: Option<i64>,
) -> Result<()> {
    let _ = record_pool_route_success_inner(
        pool,
        account_id,
        request_started_at_utc,
        sticky_key,
        None,
        invoke_id,
        attempt_id,
        None,
    )
    .await?;
    record_pool_route_success_capability_observations(
        pool,
        account_id,
        endpoint,
        image_intent,
        None,
    )
    .await
}

async fn record_pool_route_success_capability_observations(
    pool: &Pool<Sqlite>,
    account_id: i64,
    endpoint: &str,
    image_intent: ImageIntent,
    codex_imagegen_rewrite: Option<&Value>,
) -> Result<()> {
    let requirements =
        RequestCapabilityRequirements::from_endpoint_and_image_intent(endpoint, image_intent);
    if requirements.response_endpoint {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::ResponseEndpoint,
            CapabilitySupport::Supported,
            Some(UpstreamCapabilityAxis::ResponseEndpoint.success_reason()),
        )
        .await?;
    }
    if requirements.chat_completions_endpoint {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::ChatCompletionsEndpoint,
            CapabilitySupport::Supported,
            Some(UpstreamCapabilityAxis::ChatCompletionsEndpoint.success_reason()),
        )
        .await?;
    }
    if requirements.image_endpoint {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::ImageEndpoint,
            CapabilitySupport::Supported,
            Some(UpstreamCapabilityAxis::ImageEndpoint.success_reason()),
        )
        .await?;
    }
    if requirements.response_image_tool {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::ResponseImageTool,
            CapabilitySupport::Supported,
            Some(UpstreamCapabilityAxis::ResponseImageTool.success_reason()),
        )
        .await?;
    }
    if requirements.standalone_search {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::StandaloneSearch,
            CapabilitySupport::Supported,
            Some(UpstreamCapabilityAxis::StandaloneSearch.success_reason()),
        )
        .await?;
    }
    if crate::codex_imagegen_audit_has_canonical_namespace(codex_imagegen_rewrite) {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::CodexImagegen,
            CapabilitySupport::Supported,
            Some(UpstreamCapabilityAxis::CodexImagegen.success_reason()),
        )
        .await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexImagegenRetestClaim {
    NotNeeded,
    Claimed,
    AlreadyClaimed,
}

pub(crate) async fn claim_codex_imagegen_supported_retest_override(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<CodexImagegenRetestClaim> {
    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_codex_imagegen_capability_override = NULL,
            updated_at = ?2
        WHERE id = ?1
          AND policy_codex_imagegen_capability_override = 'supported'
        "#,
    )
    .bind(account_id)
    .bind(format_utc_iso(Utc::now()))
    .execute(pool)
    .await?;
    if result.rows_affected() == 1 {
        return Ok(CodexImagegenRetestClaim::Claimed);
    }

    let capability: Option<String> = sqlx::query_scalar(
        "SELECT codex_imagegen_capability FROM pool_upstream_accounts WHERE id = ?",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    Ok(if capability.as_deref() == Some("unsupported") {
        CodexImagegenRetestClaim::AlreadyClaimed
    } else {
        CodexImagegenRetestClaim::NotNeeded
    })
}

pub(crate) async fn record_capability_observation(
    pool: &Pool<Sqlite>,
    account_id: i64,
    axis: UpstreamCapabilityAxis,
    capability: CapabilitySupport,
    reason: Option<&str>,
) -> Result<()> {
    if matches!(capability, CapabilitySupport::Unknown) {
        return Ok(());
    }
    let now_iso = format_utc_iso(Utc::now());
    let api_key_only_filter = if matches!(axis, UpstreamCapabilityAxis::StandaloneSearch) {
        " AND kind = 'api_key_codex'"
    } else {
        ""
    };
    let statement = format!(
        r#"
        UPDATE pool_upstream_accounts
        SET {observed_column} = ?2,
            {observed_at_column} = ?3,
            {reason_column} = ?4,
            updated_at = ?3
        WHERE id = ?1{api_key_only_filter}
        "#,
        observed_column = axis.observed_column(),
        observed_at_column = axis.observed_at_column(),
        reason_column = axis.reason_column(),
        api_key_only_filter = api_key_only_filter,
    );
    sqlx::query(&statement)
        .bind(account_id)
        .bind(capability.as_str())
        .bind(&now_iso)
        .bind(reason)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn record_pool_route_http_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    record_pool_route_http_failure_with_image_intent(
        pool,
        account_id,
        account_kind,
        single_account_rotation_enabled,
        sticky_key,
        status,
        error_message,
        invoke_id,
        ImageIntent::Unknown,
    )
    .await
}

pub(crate) async fn record_pool_route_http_failure_with_image_intent(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
    image_intent: ImageIntent,
) -> Result<()> {
    record_pool_route_http_failure_with_image_intent_inner(
        pool,
        account_id,
        account_kind,
        single_account_rotation_enabled,
        sticky_key,
        status,
        error_message,
        invoke_id,
        "",
        image_intent,
        None,
        None,
        None,
    )
    .await
    .map(|_| ())
}

pub(crate) async fn record_pool_route_http_failure_for_endpoint_with_image_intent(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
) -> Result<()> {
    record_pool_route_http_failure_with_image_intent_inner(
        pool,
        account_id,
        account_kind,
        single_account_rotation_enabled,
        sticky_key,
        status,
        error_message,
        invoke_id,
        endpoint,
        image_intent,
        None,
        None,
        None,
    )
    .await
    .map(|_| ())
}

async fn record_pool_route_http_failure_with_image_intent_inner(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
    prompt_cache_key: Option<&str>,
) -> Result<bool> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let requirements =
        RequestCapabilityRequirements::from_endpoint_and_image_intent(endpoint, image_intent);
    if requirements.response_endpoint
        && classify_response_endpoint_capability_observation(status, Some(error_message))
            == CapabilitySupport::Unsupported
    {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::ResponseEndpoint,
            CapabilitySupport::Unsupported,
            Some(error_message),
        )
        .await?;
    }
    if requirements.chat_completions_endpoint
        && classify_chat_completions_capability_observation(status, Some(error_message))
            == CapabilitySupport::Unsupported
    {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::ChatCompletionsEndpoint,
            CapabilitySupport::Unsupported,
            Some(error_message),
        )
        .await?;
    }
    if requirements.image_endpoint
        && classify_image_endpoint_capability_observation(status, Some(error_message))
            == CapabilitySupport::Unsupported
    {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::ImageEndpoint,
            CapabilitySupport::Unsupported,
            Some(error_message),
        )
        .await?;
    }
    if requirements.response_image_tool
        && classify_response_image_tool_capability_observation(status, Some(error_message))
            == CapabilitySupport::Unsupported
    {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::ResponseImageTool,
            CapabilitySupport::Unsupported,
            Some(error_message),
        )
        .await?;
    }
    if requirements.standalone_search
        && classify_standalone_search_capability_observation(status, Some(error_message))
            == CapabilitySupport::Unsupported
    {
        record_capability_observation(
            pool,
            account_id,
            UpstreamCapabilityAxis::StandaloneSearch,
            CapabilitySupport::Unsupported,
            Some(error_message),
        )
        .await?;
    }
    if route_http_failure_is_retryable_responses_overload(status, error_message) {
        if account_kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
            record_api_key_temporary_model_failure_or_diagnostic(
                pool,
                account_id,
                sticky_key,
                error_message,
                PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
                UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
                status,
                invoke_id,
                attempt_id,
            )
            .await?;
            return Ok(false);
        }
        return record_pool_route_retryable_overload_failure_inner(
            pool,
            account_id,
            sticky_key,
            error_message,
            invoke_id,
            attempt_id,
        )
        .await
        .map(|_| false);
    }

    let explicit_model_failure = if account_kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        match attempt_id {
            Some(attempt_id) => {
                attempt_has_explicit_model_failure(pool, attempt_id, status, Some(error_message))
                    .await?
            }
            None => is_explicit_model_failure(status, Some(error_message)),
        }
    } else {
        false
    };
    let classification = classify_pool_account_http_failure(account_kind, status, error_message);
    let api_key_temporary_http_failure = account_kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
        && classification.disposition != UpstreamAccountFailureDisposition::HardUnavailable
        && (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error());
    if account_kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
        && let Some(model) = extract_unsupported_model_from_route_error(status, error_message)
    {
        ensure_account_has_unsupported_model_tag(pool, account_id, &model).await?;
    }
    if !api_key_temporary_http_failure && let Some(attempt_id) = attempt_id {
        record_model_route_failure_from_attempt(
            pool,
            account_id,
            attempt_id,
            status,
            Some(error_message),
            Some(classification.failure_kind),
        )
        .await?;
    }
    if account_kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
        && explicit_model_failure
        && !api_key_temporary_http_failure
    {
        return Ok(false);
    }

    if account_kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
        && classification.disposition != UpstreamAccountFailureDisposition::HardUnavailable
    {
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            record_api_key_temporary_model_failure_or_diagnostic(
                pool,
                account_id,
                sticky_key,
                error_message,
                classification.failure_kind,
                classification.reason_code,
                status,
                invoke_id,
                attempt_id,
            )
            .await?;
        } else {
            let now_iso = format_utc_iso_millis(Utc::now());
            record_suppressed_pool_route_status_change(
                pool,
                account_id,
                error_message,
                sticky_key,
                classification.failure_kind,
                classification.reason_code,
                status,
                invoke_id,
                &now_iso,
                attempt_id,
            )
            .await?;
        }
        return Ok(false);
    }
    match classification.disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => {
            let now_iso = format_utc_iso_millis(Utc::now());
            let mut sticky_route_cleared = false;
            if !account_status_change_reason_is_enabled(
                pool,
                account_id,
                classification.reason_code,
            )
            .await?
            {
                record_suppressed_pool_route_status_change(
                    pool,
                    account_id,
                    error_message,
                    sticky_key,
                    classification.failure_kind,
                    classification.reason_code,
                    status,
                    invoke_id,
                    &now_iso,
                    attempt_id,
                )
                .await?;
                return Ok(false);
            }
            if is_scope_permission_error_message(error_message)
                && let Some(sticky_key) = sticky_key
            {
                sticky_route_cleared = delete_sticky_route_if_matches_with_cause(
                    pool,
                    sticky_key,
                    account_id,
                    sticky_affinity_generation,
                    attempt_id,
                    Some(i64::from(status.as_u16())),
                    Some(classification.reason_code),
                    prompt_cache_key,
                    &now_iso,
                )
                .await?;
            }
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_error = ?3,
                    last_error_at = ?4,
                    last_route_failure_at = ?4,
                    last_route_failure_kind = ?5,
                    cooldown_until = NULL,
                    consecutive_route_failures = consecutive_route_failures + 1,
                    temporary_route_failure_streak_started_at = NULL,
                    updated_at = ?4
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(
                classification
                    .next_account_status
                    .unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR),
            )
            .bind(error_message)
            .bind(&now_iso)
            .bind(classification.failure_kind)
            .execute(pool)
            .await?;
            record_upstream_account_action_for_attempt(
                pool,
                account_id,
                UpstreamAccountActionPayload {
                    action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                    source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                    reason_code: Some(classification.reason_code),
                    reason_message: Some(error_message),
                    http_status: Some(status),
                    failure_kind: Some(classification.failure_kind),
                    invoke_id,
                    sticky_key,
                    occurred_at: &now_iso,
                },
                attempt_id,
            )
            .await?;
            Ok(sticky_route_cleared)
        }
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => {
            let mut sticky_route_cleared = false;
            let base_secs = if status == StatusCode::TOO_MANY_REQUESTS {
                15
            } else {
                5
            };
            let next_account_status = if account_kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
                && classification.failure_kind
                    == FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
            {
                UPSTREAM_ACCOUNT_STATUS_ERROR
            } else {
                UPSTREAM_ACCOUNT_STATUS_ACTIVE
            };
            let applied_status_change = apply_pool_route_cooldown_failure(
                pool,
                account_id,
                next_account_status,
                sticky_key,
                error_message,
                classification.failure_kind,
                classification.reason_code,
                status,
                base_secs,
                invoke_id,
                attempt_id,
            )
            .await?;
            if applied_status_change
                && single_account_rotation_enabled
                && status == StatusCode::TOO_MANY_REQUESTS
                && let Some(sticky_key) = sticky_key
            {
                let now_iso = format_utc_iso(Utc::now());
                sticky_route_cleared = delete_sticky_route_if_matches_with_cause(
                    pool,
                    sticky_key,
                    account_id,
                    sticky_affinity_generation,
                    attempt_id,
                    Some(i64::from(status.as_u16())),
                    Some(classification.reason_code),
                    prompt_cache_key,
                    &now_iso,
                )
                .await?;
            }
            Ok(sticky_route_cleared)
        }
    }
}

pub(crate) async fn record_pool_route_retryable_overload_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    record_pool_route_retryable_overload_failure_inner(
        pool,
        account_id,
        sticky_key,
        error_message,
        invoke_id,
        None,
    )
    .await
}

async fn record_pool_route_retryable_overload_failure_inner(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
) -> Result<()> {
    let account = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    if account.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        record_api_key_temporary_model_failure_or_diagnostic(
            pool,
            account_id,
            sticky_key,
            error_message,
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
            StatusCode::OK,
            invoke_id,
            attempt_id,
        )
        .await?;
        return Ok(());
    }
    apply_pool_route_cooldown_failure(
        pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        sticky_key,
        error_message,
        PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
        StatusCode::OK,
        5,
        invoke_id,
        attempt_id,
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_pool_route_transport_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    record_pool_route_transport_failure_inner(
        pool,
        account_id,
        sticky_key,
        error_message,
        invoke_id,
        None,
    )
    .await
}

pub(crate) async fn record_pool_route_transport_failure_for_model(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
    model: Option<&str>,
) -> Result<()> {
    let account = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    if account.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        record_api_key_temporary_model_failure_or_diagnostic_with_model(
            pool,
            account_id,
            sticky_key,
            error_message,
            PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
            StatusCode::BAD_GATEWAY,
            invoke_id,
            None,
            model,
        )
        .await?;
        return Ok(());
    }
    record_pool_route_transport_failure_inner(
        pool,
        account_id,
        sticky_key,
        error_message,
        invoke_id,
        None,
    )
    .await
}

async fn record_pool_route_transport_failure_inner(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
) -> Result<()> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let account = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    if account.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        record_api_key_temporary_model_failure_or_diagnostic(
            pool,
            account_id,
            sticky_key,
            error_message,
            PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
            StatusCode::BAD_GATEWAY,
            invoke_id,
            attempt_id,
        )
        .await?;
        return Ok(());
    }
    apply_pool_route_cooldown_failure(
        pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        sticky_key,
        error_message,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
        StatusCode::BAD_GATEWAY,
        5,
        invoke_id,
        attempt_id,
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_pool_route_transport_failure_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
) -> Result<()> {
    record_pool_route_transport_failure_for_attempt_with_kind(
        pool,
        account_id,
        sticky_key,
        error_message,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        invoke_id,
        attempt_id,
    )
    .await
}

pub(crate) async fn record_pool_route_transport_failure_for_attempt_with_kind(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    failure_kind: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
) -> Result<()> {
    let account = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    if account.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        record_api_key_temporary_model_failure_or_diagnostic(
            pool,
            account_id,
            sticky_key,
            error_message,
            failure_kind,
            UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
            StatusCode::BAD_GATEWAY,
            invoke_id,
            attempt_id,
        )
        .await?;
        return Ok(());
    }
    record_pool_route_transport_failure_inner(
        pool,
        account_id,
        sticky_key,
        error_message,
        invoke_id,
        attempt_id,
    )
    .await
}

pub(crate) async fn record_pool_route_retryable_overload_failure_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
) -> Result<()> {
    record_pool_route_retryable_overload_failure_inner(
        pool,
        account_id,
        sticky_key,
        error_message,
        invoke_id,
        attempt_id,
    )
    .await
}

pub(crate) async fn record_pool_route_http_failure_with_image_intent_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
    image_intent: ImageIntent,
    attempt_id: Option<i64>,
) -> Result<()> {
    record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
        pool,
        account_id,
        account_kind,
        single_account_rotation_enabled,
        sticky_key,
        status,
        error_message,
        invoke_id,
        "",
        image_intent,
        attempt_id,
        None,
    )
    .await
}

pub(crate) async fn record_pool_route_http_failure_for_endpoint_with_image_intent_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
) -> Result<()> {
    record_pool_route_http_failure_with_image_intent_inner(
        pool,
        account_id,
        account_kind,
        single_account_rotation_enabled,
        sticky_key,
        status,
        error_message,
        invoke_id,
        endpoint,
        image_intent,
        attempt_id,
        sticky_affinity_generation,
        None,
    )
    .await
    .map(|_| ())
}

pub(crate) async fn record_pool_route_http_failure_for_endpoint_with_image_intent_and_prompt_cache_key_for_attempt(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
    prompt_cache_key: Option<&str>,
) -> Result<()> {
    record_pool_route_http_failure_with_image_intent_inner(
        pool,
        account_id,
        account_kind,
        single_account_rotation_enabled,
        sticky_key,
        status,
        error_message,
        invoke_id,
        endpoint,
        image_intent,
        attempt_id,
        sticky_affinity_generation,
        prompt_cache_key,
    )
    .await
    .map(|_| ())
}

pub(crate) async fn record_pool_route_http_failure_for_endpoint_with_image_intent_and_prompt_cache_key_for_attempt_and_broadcast(
    state: &AppState,
    account_id: i64,
    account_kind: &str,
    single_account_rotation_enabled: bool,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
    endpoint: &str,
    image_intent: ImageIntent,
    attempt_id: Option<i64>,
    sticky_affinity_generation: Option<i64>,
    prompt_cache_key: Option<&str>,
) -> Result<()> {
    let sticky_route_cleared = record_pool_route_http_failure_with_image_intent_inner(
        &state.pool,
        account_id,
        account_kind,
        single_account_rotation_enabled,
        sticky_key,
        status,
        error_message,
        invoke_id,
        endpoint,
        image_intent,
        attempt_id,
        sticky_affinity_generation,
        prompt_cache_key,
    )
    .await?;
    if sticky_route_cleared
        && let Some(prompt_cache_key) = prompt_cache_key.filter(|key| sticky_key == Some(*key))
    {
        broadcast_prompt_cache_conversation_changed(state, prompt_cache_key);
    }
    Ok(())
}

pub(crate) async fn record_suppressed_pool_route_status_change(
    pool: &Pool<Sqlite>,
    account_id: i64,
    error_message: &str,
    sticky_key: Option<&str>,
    failure_kind: &str,
    reason_code: &str,
    http_status: StatusCode,
    invoke_id: Option<&str>,
    occurred_at: &str,
    attempt_id: Option<i64>,
) -> Result<()> {
    record_upstream_account_action_for_attempt_with_latest_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED,
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status: Some(http_status),
            failure_kind: Some(failure_kind),
            invoke_id,
            sticky_key,
            occurred_at,
        },
        attempt_id,
        false,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "Temporary model health records retain the complete upstream evidence contract."
)]
pub(crate) async fn record_api_key_temporary_model_failure_or_diagnostic(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: StatusCode,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
) -> Result<()> {
    record_api_key_temporary_model_failure_or_diagnostic_with_model(
        pool,
        account_id,
        sticky_key,
        error_message,
        failure_kind,
        reason_code,
        http_status,
        invoke_id,
        attempt_id,
        None,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "Temporary model health records retain the complete upstream evidence contract."
)]
async fn record_api_key_temporary_model_failure_or_diagnostic_with_model(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: StatusCode,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
    exact_model: Option<&str>,
) -> Result<()> {
    let status_change_enabled =
        account_status_change_reason_is_enabled(pool, account_id, reason_code).await?;
    let model_failure_recorded = if status_change_enabled {
        match attempt_id {
            Some(attempt_id) => {
                record_temporary_model_route_failure_from_attempt(
                    pool,
                    account_id,
                    attempt_id,
                    http_status,
                    Some(error_message),
                    Some(failure_kind),
                    reason_code,
                )
                .await?
            }
            None => match exact_model {
                Some(model) => {
                    record_temporary_model_route_failure_for_model(
                        pool,
                        account_id,
                        model,
                        http_status,
                        Some(error_message),
                        Some(failure_kind),
                        reason_code,
                    )
                    .await?
                }
                None => false,
            },
        }
    } else {
        false
    };
    if model_failure_recorded {
        return Ok(());
    }

    let now_iso = format_utc_iso(Utc::now());
    record_suppressed_pool_route_status_change(
        pool,
        account_id,
        error_message,
        sticky_key,
        failure_kind,
        reason_code,
        http_status,
        invoke_id,
        &now_iso,
        attempt_id,
    )
    .await
}

pub(crate) async fn record_account_selected(state: &AppState, account_id: i64) {
    let now_iso = format_utc_iso(Utc::now());
    state
        .pool_account_selection_runtime
        .record_selected(account_id, now_iso.clone());
    let touch = BatchedAccountSelectedTouch {
        account_id,
        selected_at: now_iso,
    };
    if !state
        .sqlite_batch_writer
        .enqueue(SqliteBatchWrite::AccountSelectedTouch(touch.clone()))
    {
        warn!(
            account_id,
            enqueue_failed_by_class = "account_selected_touch",
            business_unblocked_record_write = true,
            "account selected touch dropped by sqlite write controller"
        );
    }
}

pub(crate) async fn record_compact_support_observation(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    if !matches!(
        status,
        COMPACT_SUPPORT_STATUS_SUPPORTED | COMPACT_SUPPORT_STATUS_UNSUPPORTED
    ) {
        return Ok(());
    }
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET compact_support_status = ?2,
            compact_support_observed_at = ?3,
            compact_support_reason = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(now_iso)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn apply_pool_route_cooldown_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    next_account_status: &str,
    sticky_key: Option<&str>,
    error_message: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: StatusCode,
    base_secs: i64,
    invoke_id: Option<&str>,
    attempt_id: Option<i64>,
) -> Result<bool> {
    let row = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    let now_iso = format_utc_iso(Utc::now());
    if !account_status_change_reason_is_enabled(pool, account_id, reason_code).await? {
        record_suppressed_pool_route_status_change(
            pool,
            account_id,
            error_message,
            sticky_key,
            failure_kind,
            reason_code,
            http_status,
            invoke_id,
            &now_iso,
            attempt_id,
        )
        .await?;
        return Ok(false);
    }
    let now = Utc::now();
    let continuing_temporary_streak = row.consecutive_route_failures > 0
        && route_failure_kind_is_temporary(row.last_route_failure_kind.as_deref());
    let next_failures = if continuing_temporary_streak {
        row.consecutive_route_failures.max(0) + 1
    } else {
        1
    };
    let streak_started_at = if continuing_temporary_streak {
        row.temporary_route_failure_streak_started_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .or_else(|| {
                row.last_route_failure_at
                    .as_deref()
                    .and_then(parse_rfc3339_utc)
            })
            .unwrap_or(now)
    } else {
        now
    };
    let should_start_cooldown = next_failures >= POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD
        || now.signed_duration_since(streak_started_at).num_seconds()
            >= POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS;
    let exponent = (next_failures - 1).clamp(0, 5) as u32;
    let cooldown_secs =
        (base_secs * (1_i64 << exponent)).min(POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS);
    let now_iso = format_utc_iso_millis(now);
    let streak_started_at_iso = format_utc_iso_millis(streak_started_at);
    let cooldown_until = should_start_cooldown
        .then(|| format_utc_iso_millis(now + ChronoDuration::seconds(cooldown_secs)));
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = ?3,
            last_error_at = ?4,
            last_route_failure_at = ?4,
            last_route_failure_kind = ?5,
            cooldown_until = ?6,
            consecutive_route_failures = ?7,
            temporary_route_failure_streak_started_at = ?8,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(next_account_status)
    .bind(error_message)
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(cooldown_until)
    .bind(next_failures)
    .bind(streak_started_at_iso)
    .execute(pool)
    .await?;
    record_upstream_account_action_for_attempt(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: if should_start_cooldown {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED
            } else {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE
            },
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status: Some(http_status),
            failure_kind: Some(failure_kind),
            invoke_id,
            sticky_key,
            occurred_at: &now_iso,
        },
        attempt_id,
    )
    .await?;
    Ok(true)
}
