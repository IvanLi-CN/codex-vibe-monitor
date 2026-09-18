type InvocationHourlyOverallMap = BTreeMap<(i64, String), InvocationHourlyRollupDelta>;
type InvocationHourlyFailureMap = BTreeMap<(i64, String, String, i64, String), i64>;
type InvocationHourlyPerfMap = BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>;
type InvocationHourlyPromptCacheMap = BTreeMap<(i64, String, String), KeyedConversationHourlyDelta>;
type InvocationHourlyPromptCacheAccountMap = BTreeMap<
    (i64, String, String, String, Option<i64>, Option<String>),
    KeyedConversationHourlyDelta,
>;
type InvocationHourlyAccountUsageMap = BTreeMap<(i64, i64), UpstreamAccountUsageHourlyDelta>;
type InvocationHourlyBreakdownMap = BTreeMap<
    (i64, String, String, Option<i64>, String, String),
    UpstreamAccountUsageBreakdownHourlyDelta,
>;
type InvocationHourlyStatsMap = BTreeMap<(i64, String, i64), UpstreamAccountStatsDelta>;
type InvocationHourlyStickyMap = BTreeMap<(i64, i64, String), KeyedConversationHourlyDelta>;

#[derive(Debug, Default)]
struct InvocationHourlyRollupTargets {
    overall: bool,
    failures: bool,
    perf: bool,
    prompt_cache: bool,
    prompt_cache_upstream_accounts: bool,
    upstream_account_usage: bool,
    upstream_account_usage_breakdown: bool,
    upstream_account_stats_hourly: bool,
    upstream_account_activity_v2: bool,
    upstream_account_stats_minute: bool,
    sticky_keys: bool,
}

impl InvocationHourlyRollupTargets {
    fn from_targets(targets: &[&str]) -> Self {
        Self {
            overall: targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS),
            failures: targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES),
            perf: targets.contains(&HOURLY_ROLLUP_TARGET_PROXY_PERF),
            prompt_cache: targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE),
            prompt_cache_upstream_accounts: targets
                .contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS),
            upstream_account_usage: targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE),
            upstream_account_usage_breakdown: targets
                .contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN),
            upstream_account_stats_hourly: targets
                .contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY),
            upstream_account_activity_v2: targets
                .contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2),
            upstream_account_stats_minute: targets
                .contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE),
            sticky_keys: targets.contains(&HOURLY_ROLLUP_TARGET_STICKY_KEYS),
        }
    }
}

#[derive(Debug, Default)]
struct InvocationHourlyRollupDeltas {
    overall: InvocationHourlyOverallMap,
    failures: InvocationHourlyFailureMap,
    perf: InvocationHourlyPerfMap,
    prompt_cache: InvocationHourlyPromptCacheMap,
    prompt_cache_upstream_accounts: InvocationHourlyPromptCacheAccountMap,
    upstream_account_usage: InvocationHourlyAccountUsageMap,
    upstream_account_usage_breakdown: InvocationHourlyBreakdownMap,
    upstream_account_stats_hourly: InvocationHourlyStatsMap,
    upstream_account_activity_v2: InvocationHourlyStatsMap,
    upstream_account_stats_minute: InvocationHourlyStatsMap,
    sticky_keys: InvocationHourlyStickyMap,
}

fn collect_invocation_hourly_rollup_deltas(
    rows: &[InvocationHourlySourceRecord],
    targets: &InvocationHourlyRollupTargets,
) -> Result<InvocationHourlyRollupDeltas> {
    let mut deltas = InvocationHourlyRollupDeltas::default();
    for row in rows {
        accumulate_invocation_hourly_rollup_row(&mut deltas, targets, row)?;
    }
    Ok(deltas)
}

fn accumulate_invocation_hourly_rollup_row(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
) -> Result<()> {
    let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
    accumulate_invocation_hourly_overall_delta(deltas, targets, row)?;
    accumulate_invocation_hourly_failure_delta(deltas, targets, row, bucket_start_epoch);
    accumulate_invocation_hourly_perf_delta(deltas, targets, row, bucket_start_epoch);
    accumulate_invocation_hourly_prompt_cache_deltas(deltas, targets, row, bucket_start_epoch);
    accumulate_invocation_hourly_usage_delta(deltas, targets, row, bucket_start_epoch);
    accumulate_invocation_hourly_breakdown_delta(deltas, targets, row)?;
    accumulate_invocation_hourly_stats_deltas(deltas, targets, row, bucket_start_epoch)?;
    accumulate_invocation_hourly_activity_delta(deltas, targets, row, bucket_start_epoch);
    accumulate_invocation_hourly_sticky_delta(deltas, targets, row, bucket_start_epoch);
    Ok(())
}

fn accumulate_invocation_hourly_overall_delta(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
) -> Result<()> {
    if targets.overall {
        accumulate_invocation_hourly_overall_rollups(
            &mut deltas.overall,
            std::slice::from_ref(row),
        )?;
    }
    Ok(())
}

fn accumulate_invocation_hourly_failure_delta(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
    bucket_start_epoch: i64,
) {
    if !targets.failures {
        return;
    }
    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    if invocation_status_counts_toward_terminal_totals(row.status.as_deref())
        && classification.failure_class != FailureClass::None
    {
        let error_category = categorize_error(row.error_message.as_deref().unwrap_or_default());
        *deltas
            .failures
            .entry((
                bucket_start_epoch,
                row.source.clone(),
                classification.failure_class.as_str().to_string(),
                classification.is_actionable as i64,
                error_category,
            ))
            .or_default() += 1;
    }
}

fn accumulate_invocation_hourly_perf_delta(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
    bucket_start_epoch: i64,
) {
    if targets.perf && row.source == SOURCE_PROXY {
        for (stage, value_ms) in [
            (PERF_STAGE_TOTAL, row.t_total_ms),
            (PERF_STAGE_REQUEST_READ, row.t_req_read_ms),
            (PERF_STAGE_REQUEST_PARSE, row.t_req_parse_ms),
            (PERF_STAGE_UPSTREAM_CONNECT, row.t_upstream_connect_ms),
            (PERF_STAGE_UPSTREAM_FIRST_BYTE, row.t_upstream_ttfb_ms),
            (PERF_STAGE_UPSTREAM_STREAM, row.t_upstream_stream_ms),
            (PERF_STAGE_RESPONSE_PARSE, row.t_resp_parse_ms),
            (PERF_STAGE_PERSISTENCE, row.t_persist_ms),
        ] {
            record_proxy_perf_stage_sample(&mut deltas.perf, bucket_start_epoch, stage, value_ms);
        }
    }
}

fn accumulate_invocation_hourly_prompt_cache_deltas(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
    bucket_start_epoch: i64,
) {
    if !(targets.prompt_cache || targets.prompt_cache_upstream_accounts) {
        return;
    }
    let Some(prompt_cache_key) = prompt_cache_key_from_payload(row.payload.as_deref()) else {
        return;
    };
    let is_success_like = invocation_hourly_row_is_success_like(row);
    if targets.prompt_cache {
        let entry = keyed_conversation_delta(
            &mut deltas.prompt_cache,
            bucket_start_epoch,
            &row.source,
            &prompt_cache_key,
            &row.occurred_at,
        );
        add_keyed_conversation_row(entry, row, is_success_like);
    }
    if targets.prompt_cache_upstream_accounts {
        accumulate_invocation_hourly_prompt_cache_account_delta(
            &mut deltas.prompt_cache_upstream_accounts,
            row,
            bucket_start_epoch,
            prompt_cache_key,
            is_success_like,
        );
    }
}

fn invocation_hourly_row_is_success_like(row: &InvocationHourlySourceRecord) -> bool {
    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    invocation_status_is_success_like(row.status.as_deref(), row.error_message.as_deref())
        && classification.failure_class == FailureClass::None
}

fn add_keyed_conversation_row(
    entry: &mut KeyedConversationHourlyDelta,
    row: &InvocationHourlySourceRecord,
    is_success_like: bool,
) {
    entry.request_count += 1;
    if is_success_like {
        entry.success_count += 1;
    } else {
        entry.failure_count += 1;
    }
    entry.total_tokens += row.total_tokens.unwrap_or_default();
    entry.total_cost += row.cost.unwrap_or_default();
}

fn accumulate_invocation_hourly_prompt_cache_account_delta(
    deltas: &mut InvocationHourlyPromptCacheAccountMap,
    row: &InvocationHourlySourceRecord,
    bucket_start_epoch: i64,
    prompt_cache_key: String,
    is_success_like: bool,
) {
    let upstream_account_id = row.resolved_upstream_account_id();
    let upstream_account_name = upstream_account_name_from_payload(row.payload.as_deref());
    let rollup_key = prompt_cache_upstream_account_rollup_key(
        upstream_account_id,
        upstream_account_name.as_deref(),
    );
    let entry = deltas
        .entry((
            bucket_start_epoch,
            row.source.clone(),
            prompt_cache_key,
            rollup_key,
            upstream_account_id,
            upstream_account_name,
        ))
        .or_insert_with(|| KeyedConversationHourlyDelta {
            first_seen_at: row.occurred_at.clone(),
            last_seen_at: row.occurred_at.clone(),
            ..KeyedConversationHourlyDelta::default()
        });
    entry.first_seen_at = std::cmp::min(entry.first_seen_at.clone(), row.occurred_at.clone());
    entry.last_seen_at = std::cmp::max(entry.last_seen_at.clone(), row.occurred_at.clone());
    add_keyed_conversation_row(entry, row, is_success_like);
}

fn accumulate_invocation_hourly_usage_delta(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
    bucket_start_epoch: i64,
) {
    if !targets.upstream_account_usage {
        return;
    }
    let Some(upstream_account_id) = row.resolved_upstream_account_id() else {
        return;
    };
    let entry = deltas
        .upstream_account_usage
        .entry((bucket_start_epoch, upstream_account_id))
        .or_insert_with(|| UpstreamAccountUsageHourlyDelta {
            first_seen_at: row.occurred_at.clone(),
            last_seen_at: row.occurred_at.clone(),
            ..UpstreamAccountUsageHourlyDelta::default()
        });
    entry.first_seen_at = std::cmp::min(entry.first_seen_at.clone(), row.occurred_at.clone());
    entry.last_seen_at = std::cmp::max(entry.last_seen_at.clone(), row.occurred_at.clone());
    entry.request_count += 1;
    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    if invocation_hourly_row_is_success_like(row) {
        entry.success_count += 1;
    } else if invocation_status_counts_toward_terminal_totals(row.status.as_deref())
        && classification.failure_class != FailureClass::None
    {
        entry.failure_count += 1;
    }
    entry.total_tokens += row.total_tokens.unwrap_or_default();
    let cost = row.cost.unwrap_or_default();
    entry.total_cost += cost;
    if invocation_counts_toward_non_success_usage(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    ) {
        entry.non_success_cost += cost;
    }
    entry.input_tokens += row.input_tokens.unwrap_or_default();
    entry.output_tokens += row.output_tokens.unwrap_or_default();
    entry.cache_input_tokens += row.cache_input_tokens.unwrap_or_default();
    entry.reasoning_tokens += row.reasoning_tokens.unwrap_or_default();
}

fn accumulate_invocation_hourly_breakdown_delta(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
) -> Result<()> {
    if targets.upstream_account_usage_breakdown {
        accumulate_upstream_account_usage_breakdown_rollup(
            &mut deltas.upstream_account_usage_breakdown,
            row,
        )?;
    }
    Ok(())
}

fn accumulate_invocation_hourly_stats_deltas(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
    bucket_start_epoch: i64,
) -> Result<()> {
    if !(targets.upstream_account_stats_hourly || targets.upstream_account_stats_minute) {
        return Ok(());
    }
    let Some(upstream_account_id) = row.resolved_upstream_account_id() else {
        return Ok(());
    };
    if targets.upstream_account_stats_hourly {
        let entry = deltas
            .upstream_account_stats_hourly
            .entry((bucket_start_epoch, row.source.clone(), upstream_account_id))
            .or_insert_with(default_upstream_account_stats_delta);
        accumulate_upstream_account_stats_delta(entry, row);
    }
    if targets.upstream_account_stats_minute {
        let minute_bucket_start_epoch =
            invocation_bucket_start_epoch_for_seconds(&row.occurred_at, 60)?;
        let entry = deltas
            .upstream_account_stats_minute
            .entry((
                minute_bucket_start_epoch,
                row.source.clone(),
                upstream_account_id,
            ))
            .or_insert_with(default_upstream_account_stats_delta);
        accumulate_upstream_account_stats_delta(entry, row);
    }
    Ok(())
}

fn default_upstream_account_stats_delta() -> UpstreamAccountStatsDelta {
    UpstreamAccountStatsDelta {
        first_byte_histogram: empty_approx_histogram(),
        first_response_byte_total_histogram: empty_approx_histogram(),
        first_token_histogram: empty_approx_histogram(),
        ..UpstreamAccountStatsDelta::default()
    }
}

fn accumulate_invocation_hourly_activity_delta(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
    bucket_start_epoch: i64,
) {
    if !targets.upstream_account_activity_v2
        || !invocation_row_counts_toward_account_activity_v2(row)
    {
        return;
    }
    let upstream_account_id = row
        .resolved_upstream_account_id()
        .unwrap_or(UPSTREAM_ACCOUNT_ACTIVITY_UNASSIGNED_ID);
    let entry = deltas
        .upstream_account_activity_v2
        .entry((bucket_start_epoch, row.source.clone(), upstream_account_id))
        .or_insert_with(default_upstream_account_stats_delta);
    accumulate_upstream_account_activity_v2_delta(entry, row);
    if prompt_cache_key_from_payload(row.payload.as_deref()).is_none()
        && entry
            .latest_unkeyed_conversation_at
            .as_deref()
            .is_none_or(|latest| row.occurred_at.as_str() > latest)
    {
        entry.latest_unkeyed_conversation_at = Some(row.occurred_at.clone());
    }
}

fn accumulate_invocation_hourly_sticky_delta(
    deltas: &mut InvocationHourlyRollupDeltas,
    targets: &InvocationHourlyRollupTargets,
    row: &InvocationHourlySourceRecord,
    bucket_start_epoch: i64,
) {
    if !targets.sticky_keys {
        return;
    }
    let (Some(upstream_account_id), Some(sticky_key)) = (
        row.resolved_upstream_account_id(),
        sticky_key_from_payload(row.payload.as_deref()),
    ) else {
        return;
    };
    let entry = deltas
        .sticky_keys
        .entry((bucket_start_epoch, upstream_account_id, sticky_key))
        .or_insert_with(|| KeyedConversationHourlyDelta {
            first_seen_at: row.occurred_at.clone(),
            last_seen_at: row.occurred_at.clone(),
            ..KeyedConversationHourlyDelta::default()
        });
    entry.first_seen_at = std::cmp::min(entry.first_seen_at.clone(), row.occurred_at.clone());
    entry.last_seen_at = std::cmp::max(entry.last_seen_at.clone(), row.occurred_at.clone());
    add_keyed_conversation_row(entry, row, invocation_hourly_row_is_success_like(row));
}
