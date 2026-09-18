fn append_model_routing_attempt_timeline_filters(
    query: &mut QueryBuilder<'_, Sqlite>,
    account_id: Option<i64>,
    model: Option<&str>,
    cutoff_epoch_ms: i64,
    cursor: Option<&ModelRoutingHistoryCursor>,
    route_keys: Option<&std::collections::BTreeSet<(i64, String)>>,
    model_sql: &str,
) {
    query.push(" AND COALESCE(accounts.deleted_at, '') = ''");
    query
        .push(" AND ")
        .push("attempts.occurred_epoch_ms")
        .push(" >= ");
    query.push_bind(cutoff_epoch_ms);
    query.push(" AND ").push(model_sql).push(" IS NOT NULL");
    if let Some(account_id) = account_id {
        query.push(" AND attempts.upstream_account_id = ");
        query.push_bind(account_id);
    }
    if let Some(model) = model {
        query.push(" AND ").push(model_sql).push(" = ");
        query.push_bind(model.to_string());
    }
    append_model_routing_route_key_filter(
        query,
        route_keys,
        "attempts.upstream_account_id",
        model_sql,
    );
    if let Some(cursor) = cursor {
        query
            .push(" AND (")
            .push("attempts.occurred_epoch_ms")
            .push(" < ");
        query.push_bind(cursor.occurred_epoch_ms);
        query
            .push(" OR (")
            .push("attempts.occurred_epoch_ms")
            .push(" = ");
        query.push_bind(cursor.occurred_epoch_ms);
        query.push(" AND (1 < ");
        query.push_bind(cursor.kind_rank);
        query.push(" OR (1 = ");
        query.push_bind(cursor.kind_rank);
        query.push(" AND attempts.id < ");
        query.push_bind(cursor.id);
        query.push("))))");
    }
}

type ApiKeyGroupMigrationRow = (
    i64,
    Option<String>,
    i64,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<String>,
);

async fn load_api_key_group_migration_rows(
    pool: &Pool<Sqlite>,
) -> Result<Vec<ApiKeyGroupMigrationRow>> {
    sqlx::query_as::<_, ApiKeyGroupMigrationRow>(
        r#"
        SELECT id, group_name, is_mother, upstream_base_url,
               local_primary_limit, local_secondary_limit, local_limit_unit
        FROM pool_upstream_accounts
        WHERE kind = ?1 AND COALESCE(deleted_at, '') = ''
          AND (NULLIF(TRIM(COALESCE(group_name, '')), '') IS NOT NULL OR is_mother <> 0)
        ORDER BY id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}
