struct SummaryProjectionAllTimeHydrationContext<'a> {
    input: SummaryProjectionAllTimeAggregationInput<'a>,
    all_time_rollup_totals: HashMap<(i64, Option<i64>), StatsTotals>,
    global_all_time_source_unavailable: bool,
    account_all_time_unavailable: bool,
    global_rollup_coverage_unproven: bool,
    account_rollup_coverage_unproven: bool,
    global_non_rollup_source_blocked: bool,
    account_non_rollup_source_blocked: bool,
    global_live_tail_source_blocked: bool,
    account_live_tail_source_blocked: bool,
    batched_all_time_by_account: HashMap<i64, StatsTotals>,
}

struct SummaryProjectionAllTimePreparation {
    archive_scan_paths: SummaryProjectionAllTimeArchiveScanPaths,
    live_tail: crate::stats::BoundedLiveInvocationIds,
}

impl<'a> SummaryProjectionAllTimeHydrationContext<'a> {
    fn new(
        input: SummaryProjectionAllTimeAggregationInput<'a>,
        all_time_rollup_totals: HashMap<(i64, Option<i64>), StatsTotals>,
    ) -> Self {
        Self {
            global_all_time_source_unavailable: input.global_all_time_source_unavailable,
            account_all_time_unavailable: input.account_all_time_unavailable,
            input,
            all_time_rollup_totals,
            global_rollup_coverage_unproven: false,
            account_rollup_coverage_unproven: false,
            global_non_rollup_source_blocked: false,
            account_non_rollup_source_blocked: false,
            global_live_tail_source_blocked: false,
            account_live_tail_source_blocked: false,
            batched_all_time_by_account: HashMap::new(),
        }
    }

    fn finish(self) -> SummaryProjectionAllTimeAggregation {
        SummaryProjectionAllTimeAggregation {
            global_all_time_source_unavailable: self.global_all_time_source_unavailable,
            account_all_time_unavailable: self.account_all_time_unavailable,
            batched_all_time_by_account: self.batched_all_time_by_account,
        }
    }
}

async fn prepare_summary_projection_all_time(
    context: &mut SummaryProjectionAllTimeHydrationContext<'_>,
) -> Result<SummaryProjectionAllTimePreparation> {
    let archive_scan_paths =
        load_summary_projection_all_time_archive_scan_paths(context.input.pool).await?;
    mark_summary_projection_all_time_record_coverage(context);
    let (global_proven, account_proven, paged_account_ids) =
        load_summary_projection_all_time_scope_coverage(context).await?;
    context
        .input
        .account_ids
        .extend(paged_account_ids.iter().copied());
    context
        .input
        .account_ids_with_projection_data
        .extend(paged_account_ids);
    if context.input.account_ids.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
        return Err(anyhow!(
            "summary projection all-time account cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    context.global_rollup_coverage_unproven = !global_proven;
    context.account_rollup_coverage_unproven = !account_proven;
    context.global_non_rollup_source_blocked = context.global_all_time_source_unavailable;
    context.account_non_rollup_source_blocked = context.account_all_time_unavailable;
    if context.input.all_time_manifest_high_watermark_id.is_some() {
        *context.input.all_time_archive_admission_exceeded |= !global_proven;
        *context.input.all_time_account_manifest_admission_exceeded |= !account_proven;
    }
    if !context.input.has_any_completed_archive && context.input.live_history_admission.is_none() {
        context.global_all_time_source_unavailable = true;
        context.account_all_time_unavailable = true;
    }
    let live_tail = load_summary_projection_all_time_live_tail(context).await?;
    Ok(SummaryProjectionAllTimePreparation {
        archive_scan_paths,
        live_tail,
    })
}

fn mark_summary_projection_all_time_record_coverage(
    context: &mut SummaryProjectionAllTimeHydrationContext<'_>,
) {
    let rollup_live_cursor = context.input.rollup_live_cursor;
    let account_rollup_live_cursor = context.input.account_rollup_live_cursor;
    let exact_global_buckets = context.input.exact_global_total_rollup_buckets;
    let exact_account_buckets = context.input.exact_account_total_rollup_buckets;
    let all_time_rollup_totals = &context.all_time_rollup_totals;
    for record in context
        .input
        .records
        .iter_mut()
        .chain(context.input.current_records.iter_mut())
    {
        if !record.is_persisted_live_record {
            continue;
        }
        let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
        if record.row.id <= rollup_live_cursor
            && all_time_rollup_totals.contains_key(&(bucket, None))
            && !exact_global_buckets.contains(&bucket)
        {
            record.global_rollup_covered = true;
        }
        if account_rollup_live_cursor.is_some_and(|cursor| record.row.id <= cursor)
            && all_time_rollup_totals.contains_key(&(bucket, record.row.upstream_account_id))
            && !exact_account_buckets.contains(&bucket)
        {
            record.account_rollup_covered = true;
        }
    }
}

async fn load_summary_projection_all_time_scope_coverage(
    context: &SummaryProjectionAllTimeHydrationContext<'_>,
) -> Result<(bool, bool, HashSet<i64>)> {
    if let Some(high_watermark_id) = context.input.all_time_manifest_high_watermark_id {
        return summary_projection_paged_all_time_materialized_scope_coverage(
            context.input.pool,
            high_watermark_id,
            &context.all_time_rollup_totals,
        )
        .await;
    }
    let (global, account) = summary_projection_all_time_materialized_scope_coverage(
        context.input.all_time_archives,
        context.input.all_time_archive_replay_coverage,
        context
            .input
            .all_time_archive_account_manifest_refreshed_paths,
        context.input.all_time_archive_account_ids_by_file,
        &context.all_time_rollup_totals,
    )?;
    Ok((global, account, HashSet::new()))
}

async fn load_summary_projection_all_time_live_tail(
    context: &SummaryProjectionAllTimeHydrationContext<'_>,
) -> Result<crate::stats::BoundedLiveInvocationIds> {
    if !context.input.has_any_completed_archive {
        return Ok(crate::stats::BoundedLiveInvocationIds {
            ids: HashSet::new(),
            upper_bound_id: context.input.rollup_live_cursor,
        });
    }
    crate::stats::load_live_invocation_ids_after_id_bounded_snapshot(
        context.input.pool,
        InvocationSourceScope::All,
        context.input.rollup_live_cursor,
        summary_projection_exact_record_limit(),
    )
    .await
    .map_err(|error| anyhow!("summary projection live-tail id hydration failed: {error:?}"))
}

async fn hydrate_summary_projection_all_time_global(
    context: &mut SummaryProjectionAllTimeHydrationContext<'_>,
    preparation: &SummaryProjectionAllTimePreparation,
) -> Result<()> {
    let live_tail = &preparation.live_tail;
    if context.input.has_any_completed_archive && !live_tail.ids.is_empty() {
        if context.input.rollup_live_cursor == 0 {
            context.global_all_time_source_unavailable = true;
            context.global_live_tail_source_blocked = true;
        }
        if context.input.account_rollup_live_cursor.is_none() {
            context.account_all_time_unavailable = true;
            context.account_live_tail_source_blocked = true;
        }
    }
    let mut global_totals = load_summary_projection_all_time_global_baseline(context).await?;
    if context.input.has_any_completed_archive && context.input.rollup_live_cursor > 0 {
        global_totals = global_totals.add(
            crate::stats::query_live_invocation_totals_after_id(
                context.input.pool,
                InvocationSourceScope::All,
                context.input.rollup_live_cursor,
                live_tail.upper_bound_id,
            )
            .await
            .map_err(|error| {
                anyhow!("summary projection live-tail totals hydration failed: {error:?}")
            })?,
        );
    }
    let verified_paths = verify_summary_projection_archive_file_paths_sha256(
        &preparation.archive_scan_paths.global,
        context.input.all_time_archive_manifest_sha256,
    )?;
    let raw_recovery_succeeded = recover_summary_projection_all_time_global_archive(
        context,
        preparation,
        &live_tail.ids,
        &verified_paths,
        &mut global_totals,
    )
    .await?;
    if raw_recovery_succeeded
        && context.global_rollup_coverage_unproven
        && preparation.archive_scan_paths.global_unmaterialized_count > 0
        && !context.global_non_rollup_source_blocked
        && !context.global_live_tail_source_blocked
    {
        context.global_all_time_source_unavailable = false;
    }
    add_summary_projection_all_time_global_exact_records(context, &mut global_totals);
    publish_summary_projection_all_time_global(context, global_totals);
    Ok(())
}

async fn load_summary_projection_all_time_global_baseline(
    context: &SummaryProjectionAllTimeHydrationContext<'_>,
) -> Result<StatsTotals> {
    if !context.input.has_any_completed_archive {
        return if let Some(admission) = context.input.live_history_admission.as_ref() {
            Ok(StatsTotals::from(
                crate::stats::query_stats_row_through_id(
                    context.input.pool,
                    InvocationSourceScope::All,
                    admission.upper_bound_id,
                )
                .await
                .map_err(|error| {
                    anyhow!("summary projection live all-time hydration failed: {error:?}")
                })?,
            ))
        } else {
            Ok(StatsTotals::default())
        };
    }
    Ok(context
        .all_time_rollup_totals
        .iter()
        .filter(|((bucket, account_id), _)| {
            account_id.is_none()
                && !context
                    .input
                    .exact_global_total_rollup_buckets
                    .contains(bucket)
        })
        .fold(StatsTotals::default(), |totals, (_, value)| {
            totals.add(*value)
        }))
}

async fn recover_summary_projection_all_time_global_archive(
    context: &SummaryProjectionAllTimeHydrationContext<'_>,
    preparation: &SummaryProjectionAllTimePreparation,
    live_tail_ids: &HashSet<i64>,
    verified_paths: &[String],
    global_totals: &mut StatsTotals,
) -> Result<bool> {
    if preparation.archive_scan_paths.global_unmaterialized_count == 0 {
        return Ok(false);
    }
    let totals = crate::stats::query_unmaterialized_invocation_archive_totals_bounded_strict(
        context.input.pool,
        InvocationSourceScope::All,
        None,
        Some(live_tail_ids),
        summary_projection_exact_record_limit(),
    )
    .await;
    match totals {
        Ok(totals) => {
            require_summary_projection_archive_file_paths_sha256(
                verified_paths,
                context.input.all_time_archive_manifest_sha256,
            )?;
            *global_totals = global_totals.add(totals);
            Ok(true)
        }
        Err(error)
            if error
                .to_string()
                .starts_with("summary archive is unavailable:") =>
        {
            Ok(false)
        }
        Err(error) => Err(anyhow!(
            "summary projection global archive hydration failed: {error:?}"
        )),
    }
}

fn add_summary_projection_all_time_global_exact_records(
    context: &SummaryProjectionAllTimeHydrationContext<'_>,
    global_totals: &mut StatsTotals,
) {
    let mut identities = HashSet::<(i64, &str, &str)>::new();
    for record in context
        .input
        .records
        .iter()
        .chain(context.input.current_records.iter())
    {
        if identities.insert((
            record.row.id,
            record.row.invoke_id.as_str(),
            record.row.occurred_at.as_str(),
        )) && summary_projection_all_time_uses_global_exact_record(
            record,
            context.input.rollup_live_cursor,
        ) {
            *global_totals = global_totals.add(summary_projection_all_time_record_totals(record));
        }
    }
}

fn publish_summary_projection_all_time_global(
    context: &mut SummaryProjectionAllTimeHydrationContext<'_>,
    global_totals: StatsTotals,
) {
    if context.global_all_time_source_unavailable {
        if let Some(previous_global) = context
            .input
            .previous_all_time_by_account
            .and_then(|responses| responses.get(&None))
        {
            context
                .input
                .all_time_by_account
                .insert(None, previous_global.clone());
        } else {
            context.input.all_time_by_account.remove(&None);
        }
        return;
    }
    let mut response = global_totals.into_response();
    response.non_success_cost = Some(global_totals.non_success_cost);
    response.maintenance = Some(context.input.maintenance.clone());
    context.input.all_time_by_account.insert(None, response);
}

async fn hydrate_summary_projection_all_time_accounts(
    context: &mut SummaryProjectionAllTimeHydrationContext<'_>,
    preparation: &SummaryProjectionAllTimePreparation,
) -> Result<()> {
    let account_totals = if !context.input.has_any_completed_archive {
        if let Some(admission) = context.input.live_history_admission.as_ref() {
            load_summary_projection_live_account_totals(
                context.input.pool,
                admission.upper_bound_id,
            )
            .await?
        } else {
            HashMap::new()
        }
    } else {
        load_summary_projection_account_rollup_totals(
            context.input.pool,
            context.input.exact_account_total_rollup_buckets,
        )
        .await?
    };
    context.batched_all_time_by_account.extend(account_totals);
    if context.input.has_any_completed_archive {
        let account_live_tail = admit_summary_projection_live_tail_account_ids(
            context.input.pool,
            context.input.account_rollup_live_cursor,
        )
        .await?;
        for (account_id, totals) in load_summary_projection_live_tail_account_totals(
            context.input.pool,
            context.input.account_rollup_live_cursor,
            account_live_tail.map(|tail| tail.upper_bound_id),
        )
        .await?
        {
            let entry = context
                .batched_all_time_by_account
                .entry(account_id)
                .or_default();
            *entry = entry.add(totals);
        }
    }
    let verified_paths = verify_summary_projection_archive_file_paths_sha256(
        &preparation.archive_scan_paths.account,
        context.input.all_time_archive_manifest_sha256,
    )?;
    let account_archive_totals = load_summary_projection_all_time_account_archive_totals(
        context,
        preparation,
        &preparation.live_tail.ids,
        &verified_paths,
    )
    .await?;
    for (account_id, totals) in account_archive_totals {
        let entry = context
            .batched_all_time_by_account
            .entry(account_id)
            .or_default();
        *entry = entry.add(totals);
    }
    add_summary_projection_all_time_account_exact_records(context);
    Ok(())
}

async fn load_summary_projection_all_time_account_archive_totals(
    context: &mut SummaryProjectionAllTimeHydrationContext<'_>,
    preparation: &SummaryProjectionAllTimePreparation,
    live_tail_ids: &HashSet<i64>,
    verified_paths: &[String],
) -> Result<HashMap<i64, StatsTotals>> {
    if verified_paths.len() != preparation.archive_scan_paths.account.len() {
        context.account_all_time_unavailable = true;
        return Ok(HashMap::new());
    }
    match crate::stats::query_unmaterialized_upstream_account_archive_totals_by_account(
        context.input.pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        InvocationSourceScope::All,
        None,
        Some(live_tail_ids),
    )
    .await
    {
        Ok(totals) => {
            require_summary_projection_archive_file_paths_sha256(
                &preparation.archive_scan_paths.account,
                context.input.all_time_archive_manifest_sha256,
            )?;
            if context.account_rollup_coverage_unproven
                && !preparation.archive_scan_paths.account.is_empty()
                && !context.account_non_rollup_source_blocked
                && !context.account_live_tail_source_blocked
            {
                context.account_all_time_unavailable = false;
            }
            Ok(totals)
        }
        Err(error)
            if error
                .to_string()
                .starts_with("summary account archive is unavailable:") =>
        {
            context.account_all_time_unavailable = true;
            Ok(HashMap::new())
        }
        Err(error) => Err(anyhow!(
            "summary projection account archive hydration failed: {error:?}"
        )),
    }
}

fn add_summary_projection_all_time_account_exact_records(
    context: &mut SummaryProjectionAllTimeHydrationContext<'_>,
) {
    let mut identities = HashSet::<(i64, &str, &str)>::new();
    let has_archives = context.input.has_any_completed_archive;
    for record in context
        .input
        .records
        .iter()
        .chain(context.input.current_records.iter())
    {
        if !identities.insert((
            record.row.id,
            record.row.invoke_id.as_str(),
            record.row.occurred_at.as_str(),
        )) {
            continue;
        }
        if !has_archives && record.is_persisted_live_record {
            continue;
        }
        let Some((account_id, totals)) = summary_projection_account_record_totals(record) else {
            continue;
        };
        if !summary_projection_all_time_uses_account_exact_record(
            record,
            context.input.account_rollup_live_cursor,
        ) {
            continue;
        }
        let entry = context
            .batched_all_time_by_account
            .entry(account_id)
            .or_default();
        *entry = entry.add(totals);
    }
}
