pub(crate) async fn mark_invocation_hourly_rollup_buckets_materialized_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let mut overall_targets = HashSet::new();
    let mut upstream_account_usage_targets = HashSet::new();
    let mut sticky_targets = HashSet::new();
    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        overall_targets.insert((bucket_start_epoch, row.source.clone()));
        upstream_account_usage_targets.insert(bucket_start_epoch);
        sticky_targets.insert(bucket_start_epoch);
    }

    let live_targets = load_live_invocation_bucket_targets_tx(tx, &overall_targets).await?;
    let live_proxy_buckets = live_targets
        .iter()
        .filter_map(|(bucket_start_epoch, source)| {
            (source == SOURCE_PROXY).then_some(*bucket_start_epoch)
        })
        .collect::<HashSet<_>>();

    for (bucket_start_epoch, source) in overall_targets {
        if live_targets.contains(&(bucket_start_epoch, source.clone())) {
            continue;
        }
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        ] {
            mark_hourly_rollup_bucket_materialized_tx(tx, target, bucket_start_epoch, &source)
                .await?;
        }
        if source == SOURCE_PROXY && !live_proxy_buckets.contains(&bucket_start_epoch) {
            mark_hourly_rollup_bucket_materialized_tx(
                tx,
                HOURLY_ROLLUP_TARGET_PROXY_PERF,
                bucket_start_epoch,
                SOURCE_PROXY,
            )
            .await?;
        }
    }

    for bucket_start_epoch in upstream_account_usage_targets {
        if live_targets
            .iter()
            .any(|(live_bucket_start_epoch, _)| *live_bucket_start_epoch == bucket_start_epoch)
        {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }

    for bucket_start_epoch in sticky_targets {
        if live_proxy_buckets.contains(&bucket_start_epoch) {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_STICKY_KEYS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }

    Ok(())
}

pub(crate) async fn load_live_invocation_bucket_targets_tx(
    tx: &mut SqliteConnection,
    bucket_targets: &HashSet<(i64, String)>,
) -> Result<HashSet<(i64, String)>> {
    if bucket_targets.is_empty() {
        return Ok(HashSet::new());
    }

    let min_bucket_epoch = bucket_targets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum invocation bucket epoch"))?;
    let max_bucket_epoch = bucket_targets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum invocation bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum invocation bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum invocation bucket epoch"))?;

    let rows = sqlx::query_as::<_, InvocationBucketPresenceRow>(
        r#"
        SELECT occurred_at, source
        FROM codex_invocations
        WHERE occurred_at >= ?1
          AND occurred_at < ?2
        ORDER BY id ASC
        "#,
    )
    .bind(db_occurred_at_lower_bound(min_bucket_start))
    .bind(db_occurred_at_lower_bound(max_bucket_end))
    .fetch_all(&mut *tx)
    .await?;

    let mut live_targets = HashSet::new();
    for row in rows {
        let key = (invocation_bucket_start_epoch(&row.occurred_at)?, row.source);
        if bucket_targets.contains(&key) {
            live_targets.insert(key);
        }
    }
    Ok(live_targets)
}

pub(crate) async fn mark_forward_proxy_hourly_rollup_buckets_materialized_tx(
    tx: &mut SqliteConnection,
    rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    let mut buckets = HashSet::new();
    for row in rows {
        buckets.insert(forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)?);
    }
    for bucket_start_epoch in buckets {
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn upsert_invocation_rollups(
    tx: &mut sqlx::SqliteConnection,
    candidates: &[InvocationArchiveCandidate],
) -> Result<()> {
    let mut rollups: BTreeMap<(String, String), InvocationRollupDelta> = BTreeMap::new();
    for candidate in candidates {
        let stats_date = shanghai_day_key_from_local_naive(&candidate.occurred_at)?;
        let key = (stats_date, candidate.source.clone());
        let entry = rollups.entry(key).or_default();
        entry.total_count += 1;
        if matches!(candidate.status.as_deref(), Some("success")) {
            entry.success_count += 1;
        } else if candidate
            .status
            .as_deref()
            .is_some_and(|status| status != "success")
        {
            entry.failure_count += 1;
        }
        entry.total_tokens += candidate.total_tokens.unwrap_or_default();
        entry.total_cost += candidate.cost.unwrap_or_default();
    }

    for ((stats_date, source), delta) in rollups {
        sqlx::query(
            r#"
            INSERT INTO invocation_rollup_daily (
                stats_date,
                source,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
            ON CONFLICT(stats_date, source) DO UPDATE SET
                total_count = invocation_rollup_daily.total_count + excluded.total_count,
                success_count = invocation_rollup_daily.success_count + excluded.success_count,
                failure_count = invocation_rollup_daily.failure_count + excluded.failure_count,
                total_tokens = invocation_rollup_daily.total_tokens + excluded.total_tokens,
                total_cost = invocation_rollup_daily.total_cost + excluded.total_cost
            "#,
        )
        .bind(&stats_date)
        .bind(&source)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.total_cost)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}
