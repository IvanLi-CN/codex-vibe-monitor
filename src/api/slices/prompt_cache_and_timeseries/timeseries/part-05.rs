pub(crate) async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let base = TimeseriesTopicMaterializedBase::build(state.as_ref(), &params).await?;
    base.response(&state.proxy_runtime_invocations.snapshot())
}

async fn query_timeseries_topic_in_flight_records(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    upstream_account_id: Option<i64>,
) -> Result<Vec<InvocationAggregateRecord>, ApiError> {
    match upstream_account_id {
        Some(upstream_account_id) => {
            query_in_flight_invocation_aggregate_records_from_live_range_for_account(
                pool,
                range,
                source_scope,
                Some(snapshot_id),
                upstream_account_id,
            )
            .await
        }
        None => {
            query_in_flight_invocation_aggregate_records_from_live_range(
                pool,
                range,
                source_scope,
                Some(snapshot_id),
            )
            .await
        }
    }
}

fn add_in_flight_timeseries_records(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    records: &[InvocationAggregateRecord],
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for record in records {
        let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        add_exact_record_to_timeseries_aggregate(
            aggregates.entry(bucket_epoch).or_default(),
            record,
        );
    }
    Ok(())
}

struct RuntimeTimeseriesSnapshotOverlay<'a> {
    aggregates: &'a mut BTreeMap<i64, BucketAggregate>,
    runtime_records: &'a [ApiInvocation],
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    db_runtime_records: &'a HashMap<(String, String), InvocationAggregateRecord>,
}

fn overlay_runtime_timeseries_snapshot(
    request: RuntimeTimeseriesSnapshotOverlay<'_>,
) -> Result<(), ApiError> {
    let RuntimeTimeseriesSnapshotOverlay {
        aggregates,
        runtime_records,
        source_scope,
        upstream_account_id,
        start,
        end,
        bucket_seconds,
        reporting_tz,
        db_runtime_records,
    } = request;
    let mut stale_db_runtime_row_count = 0;
    for record in runtime_records {
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        let is_in_scope =
            source_scope != InvocationSourceScope::ProxyOnly || record.source == SOURCE_PROXY;
        let is_in_flight = prompt_shared::invocation_status_is_in_flight(record.status.as_deref());
        let is_account_match = upstream_account_id
            .is_none_or(|account_id| record.upstream_account_id == Some(account_id));
        let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let is_in_range = occurred >= start && occurred < end;
        if !is_in_scope || !is_in_flight || !is_account_match || !is_in_range {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        if let Some(db_record) = db_runtime_records.get(&key) {
            subtract_stale_db_runtime_record(
                aggregates,
                db_record,
                bucket_seconds,
                reporting_tz,
                &mut stale_db_runtime_row_count,
            )?;
        }
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        entry.in_flight_count += 1;
        entry
            .in_flight_phase_counts
            .increment_phase_name(runtime_record_live_phase(record));
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        entry.record_first_token_sample(runtime_record_first_token_ms(record));
        add_optional_token_components(
            entry,
            record.total_tokens,
            record.input_tokens,
            record.output_tokens,
            record.cache_input_tokens,
            record.reasoning_tokens,
        );
        entry.total_cost += record.cost.unwrap_or_default();
    }
    Ok(())
}
