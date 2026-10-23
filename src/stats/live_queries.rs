use super::*;

pub(crate) async fn query_stats_row(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsRow> {
    match (filter, source_scope) {
        (StatsFilter::All, InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::All, InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Since(start), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1 AND occurred_at >= ?2",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(db_occurred_at_lower_bound(start))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Since(start), InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE occurred_at >= ?1",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(db_occurred_at_lower_bound(start))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Range(start, end), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE source = ?1 AND occurred_at >= ?2 AND occurred_at < ?3",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(db_occurred_at_lower_bound(start))
                .bind(db_occurred_at_upper_bound(end))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::Range(start, end), InvocationSourceScope::All) => {
            let query = format!(
                "SELECT {} FROM codex_invocations WHERE occurred_at >= ?1 AND occurred_at < ?2",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(db_occurred_at_lower_bound(start))
                .bind(db_occurred_at_upper_bound(end))
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::ProxyOnly) => {
            let query = format!(
                "WITH recent AS ( \
                    SELECT * \
                    FROM codex_invocations \
                    WHERE source = ?1 \
                    ORDER BY occurred_at DESC \
                    LIMIT ?2 \
                ) \
                SELECT {} FROM recent",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(SOURCE_PROXY)
                .bind(limit)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::All) => {
            let query = format!(
                "WITH recent AS ( \
                    SELECT * \
                    FROM codex_invocations \
                    ORDER BY occurred_at DESC \
                    LIMIT ?1 \
                ) \
                SELECT {} FROM recent",
                stats_success_failure_select_sql()
            );
            sqlx::query_as::<_, StatsRow>(&query)
                .bind(limit)
                .fetch_one(pool)
                .await
                .map_err(Into::into)
        }
    }
}

/// Aggregate the live invocation table through a previously admitted durable id fence.
///
/// Summary projection hydration uses this instead of an unbounded `StatsFilter::All` read so
/// concurrent inserts after the bounded admission cannot expand the scan or change the snapshot
/// being published.
pub(crate) async fn query_stats_row_through_id(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    upper_bound_id: i64,
) -> Result<StatsRow> {
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(stats_success_failure_select_sql())
        .push(" FROM codex_invocations WHERE id <= ")
        .push_bind(upper_bound_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .build_query_as::<StatsRow>()
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_upstream_account_stats_row(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
    upstream_account_id: i64,
) -> Result<StatsRow> {
    let account_filter = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query.push(stats_success_failure_select_sql());
    match filter {
        StatsFilter::All => {
            query.push(" FROM codex_invocations WHERE ");
            query
                .push(account_filter)
                .push(" = ")
                .push_bind(upstream_account_id);
            if source_scope == InvocationSourceScope::ProxyOnly {
                query.push(" AND source = ").push_bind(SOURCE_PROXY);
            }
        }
        StatsFilter::Since(start) => {
            query.push(" FROM codex_invocations WHERE ");
            query
                .push(account_filter)
                .push(" = ")
                .push_bind(upstream_account_id);
            query
                .push(" AND occurred_at >= ")
                .push_bind(db_occurred_at_lower_bound(start));
            if source_scope == InvocationSourceScope::ProxyOnly {
                query.push(" AND source = ").push_bind(SOURCE_PROXY);
            }
        }
        StatsFilter::Range(start, end) => {
            query.push(" FROM codex_invocations WHERE ");
            query
                .push(account_filter)
                .push(" = ")
                .push_bind(upstream_account_id);
            query
                .push(" AND occurred_at >= ")
                .push_bind(db_occurred_at_lower_bound(start))
                .push(" AND occurred_at < ")
                .push_bind(db_occurred_at_upper_bound(end));
            if source_scope == InvocationSourceScope::ProxyOnly {
                query.push(" AND source = ").push_bind(SOURCE_PROXY);
            }
        }
        StatsFilter::RecentLimit(limit) => {
            query.push(" FROM (SELECT * FROM codex_invocations WHERE ");
            query
                .push(account_filter)
                .push(" = ")
                .push_bind(upstream_account_id);
            if source_scope == InvocationSourceScope::ProxyOnly {
                query.push(" AND source = ").push_bind(SOURCE_PROXY);
            }
            query
                .push(" ORDER BY occurred_at DESC, id DESC LIMIT ")
                .push_bind(limit)
                .push(") AS recent");
        }
    }
    query
        .build_query_as::<StatsRow>()
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}
