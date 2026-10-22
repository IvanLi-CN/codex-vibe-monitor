use super::*;

pub(crate) fn named_range_bounds(
    spec: &str,
    now: DateTime<Utc>,
    tz: Tz,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    if let Some(encoded) = spec.strip_prefix(EXPLICIT_RANGE_PREFIX) {
        let (start, end) = encoded.split_once('|')?;
        let start = DateTime::parse_from_rfc3339(start)
            .ok()?
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339(end).ok()?.with_timezone(&Utc);
        let duration = end.signed_duration_since(start);
        if duration <= ChronoDuration::zero() || duration > ChronoDuration::hours(48) {
            return None;
        }
        return Some((start, end));
    }
    match spec {
        "today" => {
            let local_date = now.with_timezone(&tz).date_naive();
            let start = start_of_local_day(now, tz);
            let next_date = local_date
                .succ_opt()
                .unwrap_or(local_date + ChronoDuration::days(1));
            Some((start, local_midnight_utc(next_date, tz)))
        }
        "yesterday" => {
            let local_date = now.with_timezone(&tz).date_naive();
            let end = local_midnight_utc(local_date, tz);
            let previous_date = local_date
                .pred_opt()
                .unwrap_or(local_date - ChronoDuration::days(1));
            Some((local_midnight_utc(previous_date, tz), end))
        }
        "previous1d" => {
            let local_date = now.with_timezone(&tz).date_naive();
            let end = local_midnight_utc(local_date - ChronoDuration::days(1), tz);
            Some((
                local_midnight_utc(local_date - ChronoDuration::days(2), tz),
                end,
            ))
        }
        "previous24h" => {
            let end = now - ChronoDuration::hours(24);
            Some((end - ChronoDuration::hours(24), end))
        }
        "recent180d" => {
            let local_date = now.with_timezone(&tz).date_naive();
            let start = local_midnight_utc(local_date - ChronoDuration::days(179), tz);
            let next_date = local_date
                .succ_opt()
                .unwrap_or(local_date + ChronoDuration::days(1));
            Some((start, local_midnight_utc(next_date, tz)))
        }
        "recent365d" => {
            let local_date = now.with_timezone(&tz).date_naive();
            let start = local_midnight_utc(local_date - ChronoDuration::days(364), tz);
            let next_date = local_date
                .succ_opt()
                .unwrap_or(local_date + ChronoDuration::days(1));
            Some((start, local_midnight_utc(next_date, tz)))
        }
        "thisWeek" => {
            let start = start_of_local_week(now, tz);
            // Week end must be computed via the next local boundary, not a fixed +7*24h.
            // This keeps correctness across DST transitions.
            let start_local_date = start.with_timezone(&tz).date_naive();
            Some((
                start,
                local_midnight_utc(start_local_date + ChronoDuration::days(7), tz),
            ))
        }
        "thisMonth" => {
            let start = start_of_local_month(now, tz);
            Some((start, start_of_next_month(start, tz)))
        }
        _ => None,
    }
}
