struct SummaryProjectionMaterializationInput<'a> {
    state: &'a AppState,
    pool: &'a Pool<Sqlite>,
    records_by_invoke_id: HashMap<String, SummaryProjectionRecord>,
    current_records_by_invoke_id: HashMap<String, SummaryProjectionRecord>,
    current_archive_compact_rollup_proven_ids: &'a HashSet<String>,
    recent_index_complete: bool,
    recent_index_overflow_at: Option<DateTime<Utc>>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: HashSet<i64>,
    archive_account_ids_by_file: &'a HashMap<String, HashSet<i64>>,
}

struct SummaryProjectionMaterialization {
    maintenance: StatsMaintenanceResponse,
    records: Vec<SummaryProjectionRecord>,
    current_records: Vec<SummaryProjectionRecord>,
    hourly_buckets: BTreeMap<i64, Vec<usize>>,
    recent_indexes: HashMap<Option<i64>, Vec<usize>>,
    recent_index_complete: bool,
    recent_index_overflow_at: Option<DateTime<Utc>>,
    account_ids: HashSet<i64>,
    active_in_progress_accounts: HashSet<i64>,
    global_in_progress: InProgressSummarySnapshot,
    account_ids_with_projection_data: HashSet<i64>,
}

struct SummaryProjectionRecordSets {
    records: Vec<SummaryProjectionRecord>,
    current_records: Vec<SummaryProjectionRecord>,
    hourly_buckets: BTreeMap<i64, Vec<usize>>,
    recent_indexes: HashMap<Option<i64>, Vec<usize>>,
    recent_index_complete: bool,
    recent_index_overflow_at: Option<DateTime<Utc>>,
}

struct SummaryProjectionMaterializationAccountsInput<'a> {
    state: &'a AppState,
    pool: &'a Pool<Sqlite>,
    records: &'a [SummaryProjectionRecord],
    current_records: &'a [SummaryProjectionRecord],
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: HashSet<i64>,
    archive_account_ids_by_file: &'a HashMap<String, HashSet<i64>>,
}

struct SummaryProjectionMaterializationAccounts {
    account_ids: HashSet<i64>,
    active_in_progress_accounts: HashSet<i64>,
    global_in_progress: InProgressSummarySnapshot,
    account_ids_with_projection_data: HashSet<i64>,
}

async fn materialize_summary_projection(
    input: SummaryProjectionMaterializationInput<'_>,
) -> Result<SummaryProjectionMaterialization> {
    let SummaryProjectionMaterializationInput {
        state,
        pool,
        records_by_invoke_id,
        current_records_by_invoke_id,
        current_archive_compact_rollup_proven_ids,
        recent_index_complete,
        recent_index_overflow_at,
        hourly_rollup_totals,
        hourly_rollup_usage,
        known_account_ids,
        archive_account_ids_by_file,
    } = input;
    let maintenance = load_stats_maintenance_response(state)
        .await
        .map_err(|error| anyhow!("summary projection maintenance hydration failed: {error:?}"))?;
    let record_sets = materialize_summary_projection_record_sets(
        records_by_invoke_id,
        current_records_by_invoke_id,
        current_archive_compact_rollup_proven_ids,
        recent_index_complete,
        recent_index_overflow_at,
        state.config.list_limit_max,
    )?;
    let SummaryProjectionMaterializationAccounts {
        account_ids,
        active_in_progress_accounts,
        global_in_progress,
        account_ids_with_projection_data,
    } = load_summary_projection_materialization_accounts(
        SummaryProjectionMaterializationAccountsInput {
            state,
            pool,
            records: &record_sets.records,
            current_records: &record_sets.current_records,
            hourly_rollup_totals,
            hourly_rollup_usage,
            known_account_ids,
            archive_account_ids_by_file,
        },
    )
    .await?;
    Ok(SummaryProjectionMaterialization {
        maintenance,
        records: record_sets.records,
        current_records: record_sets.current_records,
        hourly_buckets: record_sets.hourly_buckets,
        recent_indexes: record_sets.recent_indexes,
        recent_index_complete: record_sets.recent_index_complete,
        recent_index_overflow_at: record_sets.recent_index_overflow_at,
        account_ids,
        active_in_progress_accounts,
        global_in_progress,
        account_ids_with_projection_data,
    })
}

fn materialize_summary_projection_record_sets(
    records_by_invoke_id: HashMap<String, SummaryProjectionRecord>,
    current_records_by_invoke_id: HashMap<String, SummaryProjectionRecord>,
    current_archive_compact_rollup_proven_ids: &HashSet<String>,
    mut recent_index_complete: bool,
    mut recent_index_overflow_at: Option<DateTime<Utc>>,
    recent_limit: usize,
) -> Result<SummaryProjectionRecordSets> {
    let mut records = records_by_invoke_id.into_values().collect::<Vec<_>>();
    records.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.row.id.cmp(&right.row.id))
    });
    let mut current_records = current_records_by_invoke_id
        .into_values()
        .collect::<Vec<_>>();
    current_records.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.row.id.cmp(&right.row.id))
    });
    prune_summary_projection_current_records(
        &mut current_records,
        current_archive_compact_rollup_proven_ids,
        &mut recent_index_complete,
        &mut recent_index_overflow_at,
    );
    let hourly_buckets = build_summary_projection_hourly_buckets(&records);
    let recent_indexes = build_summary_projection_recent_indexes(&current_records, recent_limit);
    Ok(SummaryProjectionRecordSets {
        records,
        current_records,
        hourly_buckets,
        recent_indexes,
        recent_index_complete,
        recent_index_overflow_at,
    })
}

fn prune_summary_projection_current_records(
    current_records: &mut Vec<SummaryProjectionRecord>,
    current_archive_compact_rollup_proven_ids: &HashSet<String>,
    recent_index_complete: &mut bool,
    recent_index_overflow_at: &mut Option<DateTime<Utc>>,
) {
    if current_records.len() <= summary_projection_exact_record_limit() {
        return;
    }
    let first_retained = current_records.len() - summary_projection_exact_record_limit();
    *recent_index_complete = false;
    let newly_pruned_at = current_records[..first_retained]
        .iter()
        .filter(|record| {
            !current_archive_compact_rollup_proven_ids
                .contains(&summary_projection_record_identity_key(record))
        })
        .map(|record| record.occurred_at)
        .max();
    if let Some(newly_pruned_at) = newly_pruned_at {
        *recent_index_overflow_at = Some(
            recent_index_overflow_at
                .map(|existing| existing.max(newly_pruned_at))
                .unwrap_or(newly_pruned_at),
        );
    }
    current_records.drain(..first_retained);
}

fn build_summary_projection_hourly_buckets(
    records: &[SummaryProjectionRecord],
) -> BTreeMap<i64, Vec<usize>> {
    let mut hourly_buckets = BTreeMap::<i64, Vec<usize>>::new();
    for (index, record) in records.iter().enumerate() {
        hourly_buckets
            .entry(align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0))
            .or_default()
            .push(index);
    }
    hourly_buckets
}

fn build_summary_projection_recent_indexes(
    current_records: &[SummaryProjectionRecord],
    recent_limit: usize,
) -> HashMap<Option<i64>, Vec<usize>> {
    let mut recent_indexes = HashMap::<Option<i64>, Vec<usize>>::new();
    for index in (0..current_records.len()).rev() {
        let account_id = current_records[index].row.upstream_account_id;
        for key in std::iter::once(None).chain(account_id.map(Some)) {
            let index_entries = recent_indexes.entry(key).or_default();
            if index_entries.len() < recent_limit {
                index_entries.push(index);
            }
        }
    }
    recent_indexes
}

async fn load_summary_projection_materialization_accounts(
    input: SummaryProjectionMaterializationAccountsInput<'_>,
) -> Result<SummaryProjectionMaterializationAccounts> {
    let SummaryProjectionMaterializationAccountsInput {
        state,
        pool,
        records,
        current_records,
        hourly_rollup_totals,
        hourly_rollup_usage,
        mut known_account_ids,
        archive_account_ids_by_file,
    } = input;
    known_account_ids.extend(
        records
            .iter()
            .filter_map(|record| record.row.upstream_account_id)
            .filter(|account_id| *account_id > 0),
    );
    known_account_ids.extend(
        current_records
            .iter()
            .filter_map(|record| record.row.upstream_account_id)
            .filter(|account_id| *account_id > 0),
    );
    known_account_ids.extend(load_summary_projection_durable_account_ids(pool).await?);
    let mut active_in_progress_accounts = HashSet::new();
    collect_summary_projection_record_in_progress_accounts(
        records,
        &mut active_in_progress_accounts,
    );
    collect_summary_projection_runtime_in_progress_accounts(
        state,
        &mut active_in_progress_accounts,
    );
    known_account_ids.extend(active_in_progress_accounts.iter().copied());
    ensure_summary_projection_account_budget(&known_account_ids)?;
    let global_in_progress =
        load_in_progress_summary_snapshot(state, InvocationSourceScope::All, None)
            .await
            .map_err(|error| {
                anyhow!("summary projection in-progress hydration failed: {error:?}")
            })?;
    let account_ids_with_projection_data = summary_projection_account_ids_with_projection_data(
        records,
        current_records,
        hourly_rollup_totals,
        hourly_rollup_usage,
        archive_account_ids_by_file,
    );
    Ok(SummaryProjectionMaterializationAccounts {
        account_ids: known_account_ids,
        active_in_progress_accounts,
        global_in_progress,
        account_ids_with_projection_data,
    })
}

fn collect_summary_projection_record_in_progress_accounts(
    records: &[SummaryProjectionRecord],
    active_accounts: &mut HashSet<i64>,
) {
    for record in records {
        if matches!(
            normalized_runtime_text(Some(record.row.status.as_str())).as_str(),
            "running" | "pending"
        ) && let Some(account_id) = record.row.upstream_account_id
        {
            active_accounts.insert(account_id);
        }
    }
}

fn collect_summary_projection_runtime_in_progress_accounts(
    state: &AppState,
    active_accounts: &mut HashSet<i64>,
) {
    for runtime_record in state.proxy_runtime_invocations.snapshot() {
        if runtime_in_flight_record_matches_filters(
            &runtime_record,
            &InvocationRecordsFilters::default(),
            InvocationSourceScope::All,
        ) && let Some(account_id) = runtime_record.upstream_account_id
        {
            active_accounts.insert(account_id);
        }
    }
}

fn ensure_summary_projection_account_budget(account_ids: &HashSet<i64>) -> Result<()> {
    if account_ids.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
        return Err(anyhow!(
            "summary projection account cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    Ok(())
}

fn summary_projection_account_ids_with_projection_data(
    records: &[SummaryProjectionRecord],
    current_records: &[SummaryProjectionRecord],
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    archive_account_ids_by_file: &HashMap<String, HashSet<i64>>,
) -> HashSet<i64> {
    let mut account_ids = records
        .iter()
        .filter_map(|record| record.row.upstream_account_id)
        .filter(|account_id| *account_id > 0)
        .collect::<HashSet<_>>();
    account_ids.extend(
        current_records
            .iter()
            .filter_map(|record| record.row.upstream_account_id)
            .filter(|account_id| *account_id > 0),
    );
    account_ids.extend(
        hourly_rollup_totals
            .keys()
            .filter_map(|(_, account_id)| *account_id)
            .filter(|account_id| *account_id > 0),
    );
    account_ids.extend(
        hourly_rollup_usage
            .keys()
            .filter_map(|(_, account_id)| *account_id)
            .filter(|account_id| *account_id > 0),
    );
    account_ids.extend(
        archive_account_ids_by_file
            .values()
            .flat_map(|account_ids| account_ids.iter().copied())
            .filter(|account_id| *account_id > 0),
    );
    account_ids
}
