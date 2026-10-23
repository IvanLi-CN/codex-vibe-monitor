use super::*;
pub(crate) const STATS_SUCCESS_LIKE_SQL: &str = "(LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', 'warning_success') OR (LOWER(TRIM(COALESCE(status, ''))) = 'http_200' AND TRIM(COALESCE(error_message, '')) = ''))";
pub(crate) const STATS_TERMINAL_STATUS_SQL: &str =
    "(LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending'))";
pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET: &str =
    "codex_invocations_summary_rollup_v2_live_cursor";
pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET: &str =
    "codex_invocations_summary_rollup_v2_archive_cursor";
pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_ROW_CURSOR_DATASET: &str =
    "codex_invocations_summary_rollup_v2_archive_row_cursor";
pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_STARTED_DATASET: &str =
    "codex_invocations_summary_rollup_v2_started";
pub(crate) const INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET: &str =
    "codex_invocations_summary_rollup_v2_incomplete";
const INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_CURSOR_DATASET: &str =
    "codex_invocations_summary_rollup_v2_restored_live_cursor";
const INVOCATION_SUMMARY_ROLLUP_REPAIR_RESTORED_LIVE_END_DATASET: &str =
    "codex_invocations_summary_rollup_v2_restored_live_end";
const INVOCATION_SUMMARY_ROLLUP_REPAIR_SEEN_IDS_DATASET: &str =
    "codex_invocations_summary_rollup_v2_seen_ids";
const INVOCATION_SUMMARY_ROLLUP_REPAIR_CLEARED_BUCKETS_DATASET: &str =
    "codex_invocations_summary_rollup_v2_cleared_buckets";
const INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_OVERALL_TABLE: &str =
    "invocation_rollup_hourly_summary_repair_backup";
const INVOCATION_SUMMARY_ROLLUP_REPAIR_BACKUP_FAILURES_TABLE: &str =
    "invocation_failure_rollup_hourly_summary_repair_backup";
pub(crate) const MISSING_INVOCATION_ARCHIVE_REPAIR_PREFIX: &str =
    "completed invocation archive is missing during summary rollup repair";
const INVOCATION_SUMMARY_ROLLUP_REPAIR_BATCH_SIZE: i64 = 256;
const SUMMARY_ROLLUP_FORCE_REBUILD_ARCHIVE_LIMIT: usize = SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES;

fn archive_sha_log_ref(sha256: &str) -> &str {
    sha256.get(..12).unwrap_or(sha256)
}

const SUMMARY_ACCOUNT_ARCHIVE_MAX_ROWS: usize = 50_000;
const SUMMARY_ACCOUNT_ARCHIVE_MAX_BATCHES: usize = 4_096;

mod archive_loaders;
mod live_queries;
mod range_and_models;
mod repair_backfill;
mod rollup_queries;
mod rollup_ranges_and_totals;

pub(crate) use archive_loaders::*;
pub(crate) use live_queries::*;
pub(crate) use range_and_models::*;
pub(crate) use repair_backfill::*;
pub(crate) use rollup_queries::*;
pub(crate) use rollup_ranges_and_totals::*;

#[derive(Debug)]
pub(crate) enum ApiError {
    BadRequest(anyhow::Error),
    Conflict(anyhow::Error),
    Unavailable(anyhow::Error),
    Internal(anyhow::Error),
}

impl ApiError {
    pub(crate) fn bad_request<E>(err: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::BadRequest(err.into())
    }

    pub(crate) fn unavailable<E>(err: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::Unavailable(err.into())
    }

    pub(crate) fn conflict<E>(err: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::Conflict(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, err) = match self {
            ApiError::BadRequest(err) => (StatusCode::BAD_REQUEST, err),
            ApiError::Conflict(err) => (StatusCode::CONFLICT, err),
            ApiError::Unavailable(err) => (StatusCode::SERVICE_UNAVAILABLE, err),
            ApiError::Internal(err) => (StatusCode::INTERNAL_SERVER_ERROR, err),
        };
        let message = format!("{err}");
        (status, message).into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

// --- ISO8601 UTC helpers and serializers ---
pub(crate) fn format_utc_iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn format_utc_iso_millis(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn format_utc_iso_precise(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub(crate) fn parse_to_utc_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        if let Some(loc) = Shanghai.from_local_datetime(&naive).single() {
            return Some(loc.with_timezone(&Utc));
        }
        return Some(Utc.from_utc_datetime(&naive));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        if let Some(loc) = Shanghai.from_local_datetime(&naive).single() {
            return Some(loc.with_timezone(&Utc));
        }
        return Some(Utc.from_utc_datetime(&naive));
    }
    None
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_local_naive_to_utc_iso<S>(
    value: &String,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let iso = parse_to_utc_datetime(value)
        .map(format_utc_iso)
        .unwrap_or_else(|| value.clone());
    serializer.serialize_str(&iso)
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_local_or_utc_to_utc_iso<S>(
    value: &String,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serialize_local_naive_to_utc_iso(value, serializer)
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_opt_local_or_utc_to_utc_iso<S>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(s) => serialize_local_naive_to_utc_iso(s, serializer),
        None => serializer.serialize_none(),
    }
}
