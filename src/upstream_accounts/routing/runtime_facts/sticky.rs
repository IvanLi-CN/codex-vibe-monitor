use super::*;
use crate::storage::core::BackendKind;
use crate::storage::traffic::TrafficFactTransaction;
use sqlx::PgConnection;

async fn load_sticky_affinity_generation_authority(
    transaction: &mut TrafficFactTransaction,
    sticky_key: &str,
) -> Result<i64> {
    match transaction.backend_kind() {
        BackendKind::Sqlite => {
            load_sticky_affinity_generation_executor(transaction.sqlite_connection()?, sticky_key)
                .await
        }
        BackendKind::Postgres => sqlx::query_scalar::<_, i64>(
            "SELECT generation FROM pool_sticky_route_generations WHERE sticky_key = $1 LIMIT 1",
        )
        .bind(sticky_key)
        .fetch_optional(transaction.postgres_connection()?)
        .await
        .map(|generation| generation.unwrap_or_default())
        .map_err(Into::into),
    }
}

async fn load_sticky_model_generation_authority(
    transaction: &mut TrafficFactTransaction,
    sticky_key: &str,
    model_key: &str,
) -> Result<i64> {
    match transaction.backend_kind() {
        BackendKind::Sqlite => {
            load_sticky_model_generation_executor(transaction.sqlite_connection()?, sticky_key, model_key)
                .await
        }
        BackendKind::Postgres => {
            sqlx::query_scalar::<_, i64>(
                "SELECT generation FROM pool_sticky_model_route_generations WHERE sticky_key = $1 AND model_key = $2 LIMIT 1",
            )
            .bind(sticky_key)
            .bind(model_key)
            .fetch_optional(transaction.postgres_connection()?)
            .await
            .map(|generation| generation.unwrap_or_default())
            .map_err(Into::into)
        }
    }
}

async fn load_sticky_model_route_authority(
    transaction: &mut TrafficFactTransaction,
    sticky_key: &str,
    model_key: &str,
) -> Result<Option<PoolStickyRouteRow>> {
    match transaction.backend_kind() {
        BackendKind::Sqlite => {
            load_sticky_model_route_executor(
                transaction.sqlite_connection()?,
                sticky_key,
                model_key,
            )
            .await
        }
        BackendKind::Postgres => sqlx::query_as::<_, PoolStickyRouteRow>(
            "SELECT sticky_key, account_id, created_at, updated_at, last_seen_at \
             FROM pool_sticky_model_routes \
             WHERE sticky_key = $1 AND model_key = $2 LIMIT 1",
        )
        .bind(sticky_key)
        .bind(model_key)
        .fetch_optional(transaction.postgres_connection()?)
        .await
        .map_err(Into::into),
    }
}

async fn load_sticky_fallback_route_authority(
    transaction: &mut TrafficFactTransaction,
    sticky_key: &str,
) -> Result<Option<PoolStickyRouteRow>> {
    match transaction.backend_kind() {
        BackendKind::Sqlite => sqlx::query_as::<_, PoolStickyRouteRow>(
            "SELECT sticky_key, account_id, created_at, updated_at, last_seen_at \
             FROM pool_sticky_routes WHERE sticky_key = ?1 LIMIT 1",
        )
        .bind(sticky_key)
        .fetch_optional(transaction.sqlite_connection()?)
        .await
        .map_err(Into::into),
        BackendKind::Postgres => sqlx::query_as::<_, PoolStickyRouteRow>(
            "SELECT sticky_key, account_id, created_at, updated_at, last_seen_at \
             FROM pool_sticky_routes WHERE sticky_key = $1 LIMIT 1",
        )
        .bind(sticky_key)
        .fetch_optional(transaction.postgres_connection()?)
        .await
        .map_err(Into::into),
    }
}

async fn load_route_account_fence_authority(
    transaction: &mut TrafficFactTransaction,
    account_id: i64,
) -> Result<Option<String>> {
    let row = match transaction.backend_kind() {
        BackendKind::Sqlite => {
            load_upstream_account_row_conn(transaction.sqlite_connection()?, account_id).await?
        }
        BackendKind::Postgres => {
            load_upstream_account_row_postgres(transaction.postgres_connection()?, account_id)
                .await?
        }
    };
    Ok(row.as_ref().map(route_account_control_fingerprint))
}

async fn load_upstream_account_row_postgres(
    connection: &mut PgConnection,
    id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    sqlx::query_as::<_, UpstreamAccountRow>(&format!(
        "SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS} \
         FROM pool_upstream_accounts \
         WHERE id = $1 AND COALESCE(deleted_at, '') = '' LIMIT 1"
    ))
    .bind(id)
    .fetch_optional(connection)
    .await
    .map_err(Into::into)
}

pub(super) async fn load_projection(
    state: &AppState,
    runtime: Option<&RuntimeRouteFacts>,
    key: &str,
    model: Option<&str>,
    unconfirmed: bool,
) -> Result<(Option<PoolStickyRouteRow>, i64, bool)> {
    // Capture pending facts before the SQL snapshot. Facts committed meanwhile
    // are harmless to apply twice; removing them after an older SQL read isn't.
    let events = runtime.map_or_else(Vec::new, |runtime| runtime.sticky_events(key, unconfirmed));
    let model_key = normalize_sticky_model_key(model);
    let mut tx = state.traffic_authority.begin().await?;
    let mut epoch = load_sticky_affinity_generation_authority(&mut tx, key).await?;
    let mut model_generation = if let Some(model) = model_key.as_deref() {
        load_sticky_model_generation_authority(&mut tx, key, model).await?
    } else {
        0
    };
    let exact = if let Some(model) = model_key.as_deref() {
        load_sticky_model_route_authority(&mut tx, key, model).await?
    } else {
        None
    };
    let mut is_exact = exact.is_some();
    let mut route = if is_exact {
        exact
    } else {
        load_sticky_fallback_route_authority(&mut tx, key).await?
    };
    let mut fences = HashMap::new();
    for event in &events {
        if let std::collections::hash_map::Entry::Vacant(entry) = fences.entry(event.account_id) {
            let fence = load_route_account_fence_authority(&mut tx, event.account_id).await?;
            entry.insert(fence);
        }
    }
    tx.commit().await?;
    for event in events {
        if fences.get(&event.account_id).and_then(Option::as_deref)
            != Some(event.control_fingerprint.as_str())
        {
            continue;
        }
        let fact = event.sticky.as_ref().expect("filtered sticky fact");
        let scoped = fact.model_key.is_some();
        if scoped && fact.model_key != model_key {
            continue;
        }
        let current = if scoped {
            pack_sticky_affinity_token(epoch, model_generation)
        } else {
            epoch
        };
        if current != fact.generation_before && current != fact.generation_after {
            continue;
        }
        if scoped {
            (epoch, model_generation) = unpack_sticky_affinity_token(fact.generation_after);
        } else {
            epoch = fact.generation_after;
        }
        if !scoped && is_exact {
            continue;
        }
        let owner = route.as_ref().map(|route| route.account_id);
        if owner != fact.owner_before && owner != fact.account_id {
            continue;
        }
        route = fact.account_id.map(|account_id| PoolStickyRouteRow {
            sticky_key: key.to_owned(),
            account_id,
            created_at: fact.created_at.clone(),
            updated_at: event.observed_at.clone(),
            last_seen_at: event.observed_at.clone(),
        });
        if scoped {
            is_exact = route.is_some();
        }
    }
    Ok((
        route,
        if model_key.is_some() {
            pack_sticky_affinity_token(epoch, model_generation)
        } else {
            epoch
        },
        is_exact,
    ))
}

pub(super) async fn materialize(
    conn: &mut SqliteConnection,
    event: &RuntimeRouteEvent,
    fact: &RuntimeStickyFact,
    account: &UpstreamAccountRow,
) -> Result<()> {
    let epoch = load_sticky_affinity_generation_executor(&mut *conn, &fact.sticky_key).await?;
    let current = if let Some(model) = fact.model_key.as_deref() {
        pack_sticky_affinity_token(
            epoch,
            load_sticky_model_generation_executor(&mut *conn, &fact.sticky_key, model).await?,
        )
    } else {
        epoch
    };
    if current != fact.generation_before {
        return Ok(());
    }
    let owner: Option<i64> = if let Some(model) = fact.model_key.as_deref() {
        sqlx::query_scalar(
            "SELECT account_id FROM pool_sticky_model_routes WHERE sticky_key=?1 AND model_key=?2",
        )
        .bind(&fact.sticky_key)
        .bind(model)
        .fetch_optional(&mut *conn)
        .await?
    } else {
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key=?1")
            .bind(&fact.sticky_key)
            .fetch_optional(&mut *conn)
            .await?
    };
    // A model binding may inherit the fallback owner when it is first created.
    if fact.account_id.is_none() && owner != fact.owner_before {
        return Ok(());
    }
    match (fact.model_key.as_deref(), fact.account_id) {
        (Some(model), Some(owner)) => {
            upsert_sticky_model_route_executor(
                &mut *conn,
                &fact.sticky_key,
                model,
                owner,
                &event.observed_at,
            )
            .await?
        }
        (None, Some(owner)) => {
            upsert_sticky_route_executor(&mut *conn, &fact.sticky_key, owner, &event.observed_at)
                .await?
        }
        (Some(model), None) => {
            sqlx::query("DELETE FROM pool_sticky_model_routes WHERE sticky_key=?1 AND model_key=?2 AND account_id=?3").bind(&fact.sticky_key).bind(model).bind(fact.owner_before).execute(&mut *conn).await?;
        }
        (None, None) => {
            sqlx::query("DELETE FROM pool_sticky_routes WHERE sticky_key=?1 AND account_id=?2")
                .bind(&fact.sticky_key)
                .bind(fact.owner_before)
                .execute(&mut *conn)
                .await?;
        }
    }
    if fact.generation_before != fact.generation_after {
        if let Some(model) = fact.model_key.as_deref() {
            bump_sticky_model_generation_with_clear_cause_executor(
                &mut *conn,
                &fact.sticky_key,
                model,
                fact.account_id
                    .is_none()
                    .then_some(fact.attempt_public_id.as_deref())
                    .flatten(),
                fact.account_id
                    .is_none()
                    .then_some(event.http_status)
                    .flatten(),
                &event.observed_at,
            )
            .await?;
        } else {
            bump_sticky_affinity_generation_executor(
                &mut *conn,
                &fact.sticky_key,
                &event.observed_at,
            )
            .await?;
            if fact.account_id.is_none() {
                sqlx::query("UPDATE pool_sticky_route_generations SET last_clear_cause_attempt_public_id=?2,last_clear_cause_http_status=?3 WHERE sticky_key=?1")
                    .bind(&fact.sticky_key).bind(&fact.attempt_public_id).bind(event.http_status).execute(&mut *conn).await?;
            }
        }
    }
    if fact.account_id.is_none()
        && fact.prompt_cache_key.as_deref() == Some(fact.sticky_key.as_str())
    {
        crate::api::append_runtime_sticky_target_cleared_event_executor(
            &mut *conn,
            &fact.sticky_key,
            event.account_id,
            Some(account.display_name.clone()),
            &event.observed_at,
            event.invoke_id.clone(),
            crate::api::PromptCacheConversationOperationRoutingContext {
                reason_code: event
                    .reason_code
                    .clone()
                    .unwrap_or_else(|| "automaticStickyClear".into()),
                routing_source: fact.routing_source.clone(),
                routing_selection_audit: None,
                http_status: event
                    .http_status
                    .and_then(|status| u16::try_from(status).ok()),
                trigger_attempt_id: fact.attempt_public_id.clone(),
                causing_attempt_id: None,
                causing_http_status: None,
            },
            crate::api::PromptCacheConversationOperationRoutingScope {
                kind: if fact.model_key.is_some() {
                    "model".into()
                } else {
                    "all".into()
                },
                model_key: fact.model_key.clone(),
                request_model: None,
            },
        )
        .await?;
    }
    Ok(())
}
