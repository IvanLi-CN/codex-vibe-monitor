use super::*;

pub(crate) struct PromptCacheConversationHydrationSnapshot<'a> {
    pub(crate) snapshot_upper_bound: &'a str,
    pub(crate) snapshot_created_at_upper_bound: Option<&'a str>,
    pub(crate) snapshot_hour_start_epoch: i64,
    pub(crate) snapshot_hour_start_bound: &'a str,
    pub(crate) snapshot_boundary_row_id_ceiling: Option<i64>,
}

pub(crate) struct PromptCacheConversationHydrationRequest<'a> {
    pub(crate) source_scope: InvocationSourceScope,
    pub(crate) aggregates: Vec<PromptCacheConversationAggregateRow>,
    pub(crate) range_end: DateTime<Utc>,
    pub(crate) detail_level: PromptCacheConversationDetailLevel,
    pub(crate) recent_invocation_limit: Option<i64>,
    pub(crate) snapshot: Option<&'a PromptCacheConversationHydrationSnapshot<'a>>,
    pub(crate) runtime_overlay_records: &'a [ApiInvocation],
}

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationSnapshotFilter {
    pub(crate) snapshot_upper_bound: String,
    pub(crate) snapshot_created_at_upper_bound: Option<String>,
    pub(crate) snapshot_boundary_row_id_ceiling: Option<i64>,
}

impl PromptCacheConversationSnapshotFilter {
    pub(crate) fn snapshot_upper_bound(&self) -> &str {
        self.snapshot_upper_bound.as_str()
    }

    pub(crate) fn snapshot_created_at_upper_bound(&self) -> Option<&str> {
        self.snapshot_created_at_upper_bound.as_deref()
    }
}

pub(crate) fn push_snapshot_invocation_visibility_clause(
    query: &mut QueryBuilder<Sqlite>,
    occurred_at_expr: &str,
    id_expr: &str,
    created_at_expr: &str,
    snapshot: Option<&PromptCacheConversationSnapshotFilter>,
) {
    if let Some(snapshot) = snapshot {
        let snapshot_upper_bound = snapshot.snapshot_upper_bound().to_string();
        query.push("(");
        if let Some(created_at_upper_bound) = snapshot.snapshot_created_at_upper_bound() {
            query
                .push("julianday(")
                .push(created_at_expr)
                .push(") <= julianday(")
                .push_bind(created_at_upper_bound.to_string())
                .push(") AND ");
        }
        if let Some(row_id_ceiling) = snapshot.snapshot_boundary_row_id_ceiling {
            let boundary_occurred_at = parse_to_utc_datetime(&snapshot_upper_bound)
                .map(|upper_bound| {
                    db_occurred_at_lower_bound(upper_bound - ChronoDuration::seconds(1))
                })
                .unwrap_or_else(|| snapshot_upper_bound.clone());
            query
                .push("((")
                .push(occurred_at_expr)
                .push(" < ")
                .push_bind(boundary_occurred_at.clone())
                .push(") OR (")
                .push(occurred_at_expr)
                .push(" = ")
                .push_bind(boundary_occurred_at)
                .push(" AND ")
                .push(id_expr)
                .push(" <= ")
                .push_bind(row_id_ceiling)
                .push("))");
        } else {
            query
                .push("(")
                .push(occurred_at_expr)
                .push(" < ")
                .push_bind(snapshot_upper_bound)
                .push(")");
        }
        query.push(")");
    }
}

pub(crate) async fn hydrate_prompt_cache_conversations(
    state: &AppState,
    request: PromptCacheConversationHydrationRequest<'_>,
) -> Result<Vec<PromptCacheConversationResponse>> {
    let mut connection = state.pool.acquire().await?;
    hydrate_prompt_cache_conversations_on_connection(state, &mut connection, request).await
}

include!("hydration/part-01.rs");
include!("hydration/part-02.rs");
include!("hydration/part-03.rs");
