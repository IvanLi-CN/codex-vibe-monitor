pub(crate) async fn load_live_invocation_ids_in_range(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<std::collections::HashSet<i64>, ApiError> {
    #[derive(Debug, FromRow)]
    struct IdRow {
        id: i64,
    }

    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT id FROM codex_invocations WHERE occurred_at >= ");
    query
        .push_bind(db_occurred_at_lower_bound(start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    Ok(query
        .build_query_as::<IdRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect())
}

pub(crate) fn summary_live_augmentation_policy(
    window: &SummaryWindow,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    now: DateTime<Utc>,
) -> SummaryLiveAugmentationPolicy {
    let closed_named_window = matches!(
        window,
        SummaryWindow::Calendar(_) | SummaryWindow::PreviousFullDays(_)
    ) && range.is_some_and(|(_, end)| end < now);
    SummaryLiveAugmentationPolicy {
        include_in_progress: !closed_named_window,
        include_non_success_tokens: !closed_named_window && range.is_some(),
    }
}
