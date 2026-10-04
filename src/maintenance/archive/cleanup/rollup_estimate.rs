use super::*;

pub(crate) fn estimate_historical_rollup_pending_bucket_count(
    row: &HistoricalRollupPendingArchiveBatchRow,
) -> u64 {
    if let (Some(start), Some(end)) = (&row.coverage_start_at, &row.coverage_end_at)
        && let (Ok(start_local), Ok(end_local)) = (
            parse_shanghai_local_naive(start),
            parse_shanghai_local_naive(end),
        )
    {
        let start_utc = local_naive_to_utc(start_local, Shanghai);
        let end_utc = local_naive_to_utc(end_local, Shanghai);
        let secs = (end_utc.timestamp() - start_utc.timestamp()).max(0);
        return ((secs + 3_599) / 3_600).max(1) as u64;
    }

    let Ok(start_date) = NaiveDate::parse_from_str(&format!("{}-01", row.month_key), "%Y-%m-%d")
    else {
        return 0;
    };
    let (next_year, next_month) = if start_date.month() == 12 {
        (start_date.year() + 1, 1)
    } else {
        (start_date.year(), start_date.month() + 1)
    };
    let Some(next_month_date) = NaiveDate::from_ymd_opt(next_year, next_month, 1) else {
        return 0;
    };
    let Some(start_naive) = start_date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    let Some(end_naive) = next_month_date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    let start_utc = local_naive_to_utc(start_naive, Shanghai);
    let end_utc = local_naive_to_utc(end_naive, Shanghai);
    ((end_utc.timestamp() - start_utc.timestamp()).max(0) / 3_600) as u64
}
