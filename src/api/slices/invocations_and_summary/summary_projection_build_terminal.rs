struct SummaryProjectionTerminalCoverageInput<'a> {
    records: &'a [SummaryProjectionRecord],
    current_records: &'a [SummaryProjectionRecord],
    unavailable_exact_live_buckets: &'a BTreeSet<i64>,
    unavailable_exact_live_account_buckets: &'a HashMap<i64, BTreeSet<i64>>,
    historical_global_covered_terminal_invoke_ids: &'a HashSet<String>,
    all_time_was_fully_rebuilt: bool,
    global_all_time_source_unavailable: bool,
    previous_all_time_terminal_coverage_complete: bool,
    durable_terminal_sequence_watermark: u64,
    previous_all_time_terminal_sequence_watermark: u64,
    account_all_time_unavailable: bool,
    rebuilt_all_time_account_ids: &'a HashSet<i64>,
    previous_all_time_account_terminal_sequence_watermarks: HashMap<i64, u64>,
    previous_all_time_account_persisted_live_terminal_invoke_ids: HashMap<i64, HashSet<String>>,
}

struct SummaryProjectionTerminalCoverage {
    persisted_live_terminal_invoke_ids: HashSet<String>,
    all_time_terminal_coverage_complete: bool,
    all_time_terminal_sequence_watermark: u64,
    all_time_persisted_live_terminal_invoke_ids: HashSet<String>,
    all_time_account_terminal_sequence_watermarks: HashMap<i64, u64>,
    all_time_account_persisted_live_terminal_invoke_ids: HashMap<i64, HashSet<String>>,
}

fn build_summary_projection_terminal_coverage(
    input: SummaryProjectionTerminalCoverageInput<'_>,
) -> SummaryProjectionTerminalCoverage {
    let SummaryProjectionTerminalCoverageInput {
        records,
        current_records,
        unavailable_exact_live_buckets,
        unavailable_exact_live_account_buckets,
        historical_global_covered_terminal_invoke_ids,
        all_time_was_fully_rebuilt,
        global_all_time_source_unavailable,
        previous_all_time_terminal_coverage_complete,
        durable_terminal_sequence_watermark,
        previous_all_time_terminal_sequence_watermark,
        account_all_time_unavailable,
        rebuilt_all_time_account_ids,
        previous_all_time_account_terminal_sequence_watermarks,
        previous_all_time_account_persisted_live_terminal_invoke_ids,
    } = input;
    let persisted_live_record_has_unavailable_global_rolling_coverage =
        |record: &SummaryProjectionRecord| {
            unavailable_exact_live_buckets.contains(&align_bucket_epoch(
                record.occurred_at.timestamp(),
                3_600,
                0,
            ))
        };
    let rolling_exact_persisted_live_identities = records
        .iter()
        .filter(|record| record.is_persisted_live_record)
        .map(summary_projection_record_identity_key)
        .collect::<HashSet<_>>();
    let mut persisted_live_terminal_invoke_ids = records
        .iter()
        .chain(current_records.iter())
        .filter(|record| {
            record.is_persisted_live_record
                // The current index deliberately keeps a larger bounded prefix than a legal
                // newest-N response. An old persisted row in that prefix cannot suppress an
                // SSE terminal delta unless it is also retained in the rolling exact view; its
                // compact coverage is added separately below after both totals and usage proof.
                && rolling_exact_persisted_live_identities
                    .contains(&summary_projection_record_identity_key(record))
                && !persisted_live_record_has_unavailable_global_rolling_coverage(record)
        })
        .map(summary_projection_record_identity_key)
        .collect::<HashSet<_>>();
    persisted_live_terminal_invoke_ids.extend(
        historical_global_covered_terminal_invoke_ids
            .iter()
            .cloned(),
    );
    let (all_time_terminal_coverage_complete, all_time_terminal_sequence_watermark) =
        summary_projection_terminal_all_time_state(
            all_time_was_fully_rebuilt,
            global_all_time_source_unavailable,
            previous_all_time_terminal_coverage_complete,
            durable_terminal_sequence_watermark,
            previous_all_time_terminal_sequence_watermark,
        );
    let all_time_persisted_live_terminal_invoke_ids =
        build_summary_projection_all_time_persisted_live_terminal_ids(
            records,
            current_records,
            historical_global_covered_terminal_invoke_ids,
            all_time_was_fully_rebuilt && all_time_terminal_coverage_complete,
            &persisted_live_record_has_unavailable_global_rolling_coverage,
        );
    let (
        all_time_account_terminal_sequence_watermarks,
        all_time_account_persisted_live_terminal_invoke_ids,
    ) = if all_time_was_fully_rebuilt && !account_all_time_unavailable {
        build_rebuilt_summary_projection_account_terminal_coverage(
            records,
            current_records,
            rebuilt_all_time_account_ids,
            unavailable_exact_live_buckets,
            unavailable_exact_live_account_buckets,
            durable_terminal_sequence_watermark,
        )
    } else {
        (
            previous_all_time_account_terminal_sequence_watermarks,
            previous_all_time_account_persisted_live_terminal_invoke_ids,
        )
    };
    SummaryProjectionTerminalCoverage {
        persisted_live_terminal_invoke_ids,
        all_time_terminal_coverage_complete,
        all_time_terminal_sequence_watermark,
        all_time_persisted_live_terminal_invoke_ids,
        all_time_account_terminal_sequence_watermarks,
        all_time_account_persisted_live_terminal_invoke_ids,
    }
}

fn summary_projection_terminal_all_time_state(
    all_time_was_fully_rebuilt: bool,
    global_all_time_source_unavailable: bool,
    previous_all_time_terminal_coverage_complete: bool,
    durable_terminal_sequence_watermark: u64,
    previous_all_time_terminal_sequence_watermark: u64,
) -> (bool, u64) {
    let coverage_complete = if all_time_was_fully_rebuilt {
        !global_all_time_source_unavailable
    } else {
        previous_all_time_terminal_coverage_complete
    };
    let watermark = summary_projection_all_time_sequence_watermark(
        all_time_was_fully_rebuilt,
        coverage_complete,
        durable_terminal_sequence_watermark,
        previous_all_time_terminal_sequence_watermark,
    );
    (coverage_complete, watermark)
}

fn build_summary_projection_all_time_persisted_live_terminal_ids(
    records: &[SummaryProjectionRecord],
    current_records: &[SummaryProjectionRecord],
    historical_global_covered_terminal_invoke_ids: &HashSet<String>,
    all_time_coverage_complete: bool,
    has_unavailable_global_rolling_coverage: &dyn Fn(&SummaryProjectionRecord) -> bool,
) -> HashSet<String> {
    if all_time_coverage_complete {
        records
            .iter()
            .chain(current_records.iter())
            .filter(|record| {
                record.is_persisted_live_record && !has_unavailable_global_rolling_coverage(record)
            })
            .map(summary_projection_record_identity_key)
            .collect()
    } else {
        records
            .iter()
            .chain(current_records.iter())
            .filter(|record| record.is_persisted_live_record && record.global_rollup_covered)
            .map(summary_projection_record_identity_key)
            .chain(
                historical_global_covered_terminal_invoke_ids
                    .iter()
                    .cloned(),
            )
            .collect()
    }
}

fn build_rebuilt_summary_projection_account_terminal_coverage(
    records: &[SummaryProjectionRecord],
    current_records: &[SummaryProjectionRecord],
    rebuilt_all_time_account_ids: &HashSet<i64>,
    unavailable_exact_live_buckets: &BTreeSet<i64>,
    unavailable_exact_live_account_buckets: &HashMap<i64, BTreeSet<i64>>,
    durable_terminal_sequence_watermark: u64,
) -> (HashMap<i64, u64>, HashMap<i64, HashSet<String>>) {
    let account_watermarks = rebuilt_all_time_account_ids
        .iter()
        .map(|account_id| (*account_id, durable_terminal_sequence_watermark))
        .collect::<HashMap<_, _>>();
    let mut account_identities = HashMap::<i64, HashSet<String>>::new();
    for record in records
        .iter()
        .chain(current_records.iter())
        .filter(|record| {
            record.is_persisted_live_record
                && !unavailable_exact_live_buckets.contains(&align_bucket_epoch(
                    record.occurred_at.timestamp(),
                    3_600,
                    0,
                ))
                && record.row.upstream_account_id.is_some_and(|account_id| {
                    rebuilt_all_time_account_ids.contains(&account_id)
                        && unavailable_exact_live_account_buckets
                            .get(&account_id)
                            .is_none_or(|buckets| {
                                !buckets.contains(&align_bucket_epoch(
                                    record.occurred_at.timestamp(),
                                    3_600,
                                    0,
                                ))
                            })
                })
        })
    {
        let Some(account_id) = record.row.upstream_account_id else {
            continue;
        };
        account_identities
            .entry(account_id)
            .or_default()
            .insert(summary_projection_record_identity_key(record));
    }
    (account_watermarks, account_identities)
}
