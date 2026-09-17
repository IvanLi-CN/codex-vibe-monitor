pub(crate) async fn fetch_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StatsResponse>, ApiError> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let totals = query_combined_totals(&state.pool, StatsFilter::All, source_scope).await?;
    let mut response = totals.into_response();
    response.non_success_cost = Some(totals.non_success_cost);
    let augmentation = load_summary_live_augmentation(
        state.as_ref(),
        source_scope,
        None,
        None,
        SummaryLiveAugmentationPolicy {
            include_in_progress: true,
            include_non_success_tokens: false,
        },
        None,
    )
    .await?;
    apply_summary_live_augmentation(&mut response, augmentation);
    response.maintenance = Some(load_stats_maintenance_response(state.as_ref()).await?);
    Ok(Json(response))
}

pub(crate) async fn load_in_progress_conversation_count(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<i64, ApiError> {
    Ok(
        load_in_progress_summary_snapshot(state, source_scope, upstream_account_id)
            .await?
            .in_progress_count,
    )
}
