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
