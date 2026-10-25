use super::*;

pub(crate) fn latest_pool_attempt_phase_sql(invocation_ref: &str) -> String {
    latest_pool_attempt_phase_sql_at_snapshot(invocation_ref, None)
}

pub(crate) fn latest_pool_attempt_phase_sql_at_snapshot(
    invocation_ref: &str,
    attempt_snapshot_id: Option<i64>,
) -> String {
    let attempt_snapshot_predicate = attempt_snapshot_id
        .map(|snapshot_id| format!(" AND attempt.id <= {snapshot_id}"))
        .unwrap_or_default();
    format!(
        "(SELECT LOWER(TRIM(COALESCE(attempt.phase, ''))) \
            FROM pool_upstream_request_attempts AS attempt INDEXED BY idx_pool_upstream_request_attempts_invoke_attempt \
           WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
             AND attempt.occurred_at = {invocation_ref}.occurred_at \
             {attempt_snapshot_predicate} \
           ORDER BY attempt.attempt_index DESC, attempt.id DESC \
           LIMIT 1)"
    )
}

pub(crate) fn final_pool_attempt_first_token_ms_sql(
    attempt_ref: &str,
    invocation_ref: &str,
) -> String {
    let first_token_sql =
        sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.first_token_ms"));
    let final_attempt_evidence_sql =
        final_pool_attempt_timing_evidence_sql(attempt_ref, invocation_ref, "first_token_ms");
    format!(
        "CASE WHEN {attempt_ref}.id = (SELECT final_attempt.id \
           FROM pool_upstream_request_attempts AS final_attempt INDEXED BY idx_pool_upstream_request_attempts_invoke_attempt \
           WHERE final_attempt.invoke_id = {attempt_ref}.invoke_id \
             AND final_attempt.occurred_at = {attempt_ref}.occurred_at \
             AND LOWER(TRIM(COALESCE(final_attempt.status, ''))) <> 'budget_exhausted_final' \
           ORDER BY final_attempt.attempt_index DESC, final_attempt.id DESC \
           LIMIT 1) \
          AND ({final_attempt_evidence_sql}) \
        THEN CASE WHEN {first_token_sql} THEN {invocation_ref}.first_token_ms END END AS first_token_ms"
    )
}

pub(crate) fn final_pool_invocation_timing_sql(invocation_ref: &str, column: &str) -> String {
    final_pool_invocation_timing_sql_at_snapshot(invocation_ref, column, None)
}

pub(crate) fn final_pool_invocation_timing_sql_at_snapshot(
    invocation_ref: &str,
    column: &str,
    attempt_snapshot_id: Option<i64>,
) -> String {
    let timing_sql = match column {
        "first_token_ms" => sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.{column}")),
        "t_upstream_stream_ms" => sqlite_positive_timing_sql(&format!("{invocation_ref}.{column}")),
        _ => panic!("unsupported pool invocation timing column: {column}"),
    };
    let final_attempt_evidence_sql =
        final_pool_attempt_timing_evidence_sql("final_attempt", invocation_ref, column);
    let attempt_snapshot_predicate = attempt_snapshot_id
        .map(|snapshot_id| format!(" AND final_attempt.id <= {snapshot_id}"))
        .unwrap_or_default();
    format!(
        "CASE WHEN NOT EXISTS (SELECT 1 \
           FROM pool_upstream_request_attempts AS final_attempt INDEXED BY idx_pool_upstream_request_attempts_invoke_attempt \
          WHERE final_attempt.invoke_id = {invocation_ref}.invoke_id \
            AND final_attempt.occurred_at = {invocation_ref}.occurred_at \
            {attempt_snapshot_predicate}) \
           OR COALESCE((SELECT CASE \
             WHEN ({final_attempt_evidence_sql}) THEN 1 \
             ELSE 0 END \
           FROM pool_upstream_request_attempts AS final_attempt INDEXED BY idx_pool_upstream_request_attempts_invoke_attempt \
          WHERE final_attempt.invoke_id = {invocation_ref}.invoke_id \
            AND final_attempt.occurred_at = {invocation_ref}.occurred_at \
            {attempt_snapshot_predicate} \
            AND LOWER(TRIM(COALESCE(final_attempt.status, ''))) <> 'budget_exhausted_final' \
           ORDER BY final_attempt.attempt_index DESC, final_attempt.id DESC \
           LIMIT 1), 0) = 1 \
        THEN CASE WHEN {timing_sql} THEN {invocation_ref}.{column} END END"
    )
}

pub(crate) fn invocation_timing_sql_for_source(
    invocation_ref: &str,
    column: &str,
    use_attempt_fallback: bool,
) -> String {
    if use_attempt_fallback {
        return final_pool_invocation_timing_sql(invocation_ref, column);
    }

    let timing_sql = match column {
        "first_token_ms" => sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.{column}")),
        "t_upstream_stream_ms" => sqlite_positive_timing_sql(&format!("{invocation_ref}.{column}")),
        _ => panic!("unsupported invocation timing column: {column}"),
    };
    format!("CASE WHEN {timing_sql} THEN {invocation_ref}.{column} END")
}

fn final_pool_attempt_timing_evidence_sql(
    attempt_ref: &str,
    invocation_ref: &str,
    column: &str,
) -> String {
    let stream_measured_sql =
        sqlite_positive_timing_sql(&format!("{attempt_ref}.stream_latency_ms"));
    let zero_first_token_sql = if column == "first_token_ms" {
        let first_byte_sql =
            sqlite_nonnegative_timing_sql(&format!("{attempt_ref}.first_byte_latency_ms"));
        format!(
            " OR ({invocation_ref}.first_token_ms = 0 AND LOWER(TRIM(COALESCE({attempt_ref}.status, ''))) NOT IN ('', 'pending', 'running', 'budget_exhausted_final') AND {first_byte_sql} AND {attempt_ref}.first_byte_latency_ms = 0)"
        )
    } else {
        String::new()
    };
    let live_first_token_sql = if column == "first_token_ms" {
        let first_token_sql =
            sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.first_token_ms"));
        format!(
            " OR (LOWER(TRIM(COALESCE({invocation_ref}.status, ''))) = 'running' AND {first_token_sql} AND (LOWER(TRIM(COALESCE({attempt_ref}.status, ''))) = 'responding' OR LOWER(TRIM(COALESCE({attempt_ref}.phase, ''))) IN ('responding', 'streaming_response')))"
        )
    } else {
        String::new()
    };
    let stream_evidence_sql = format!(
        "({stream_measured_sql} AND LOWER(TRIM(COALESCE({attempt_ref}.status, ''))) NOT IN ('', 'pending', 'running', 'budget_exhausted_final'))"
    );
    if column == "first_token_ms" {
        format!("{stream_evidence_sql}{zero_first_token_sql}{live_first_token_sql}")
    } else {
        stream_evidence_sql
    }
}

fn invocation_live_phase_sql_with_first_token_predicate(
    invocation_ref: &str,
    first_token_measured_sql: &str,
) -> String {
    invocation_live_phase_sql_with_first_token_predicate_at_snapshot(
        invocation_ref,
        first_token_measured_sql,
        None,
    )
}

fn invocation_live_phase_sql_with_first_token_predicate_at_snapshot(
    invocation_ref: &str,
    first_token_measured_sql: &str,
    attempt_snapshot_id: Option<i64>,
) -> String {
    let attempt_phase_sql =
        latest_pool_attempt_phase_sql_at_snapshot(invocation_ref, attempt_snapshot_id);
    let upstream_account_id_sql = format!(
        "CASE WHEN json_valid({invocation_ref}.payload) THEN CAST(json_extract({invocation_ref}.payload, '$.upstreamAccountId') AS INTEGER) END"
    );
    format!(
        "CASE \
           WHEN LOWER(TRIM(COALESCE({invocation_ref}.status, ''))) NOT IN ('running', 'pending') THEN NULL \
           WHEN LOWER(TRIM(COALESCE({invocation_ref}.status, ''))) = 'pending' THEN '{queued}' \
           WHEN {first_token_measured_sql} THEN '{responding}' \
           WHEN {attempt_phase} IN ('connecting', 'sending_request', 'waiting_first_byte') \
             OR {upstream_account_id} IS NOT NULL \
             OR ({invocation_ref}.t_upstream_connect_ms IS NOT NULL AND {invocation_ref}.t_upstream_connect_ms > 0) \
             OR ({invocation_ref}.t_req_read_ms IS NOT NULL AND {invocation_ref}.t_req_read_ms > 0) \
             OR ({invocation_ref}.t_req_parse_ms IS NOT NULL AND {invocation_ref}.t_req_parse_ms > 0) THEN '{requesting}' \
           ELSE '{queued}' \
         END",
        attempt_phase = attempt_phase_sql,
        first_token_measured_sql = first_token_measured_sql,
        upstream_account_id = upstream_account_id_sql,
        queued = INVOCATION_LIVE_PHASE_QUEUED,
        requesting = INVOCATION_LIVE_PHASE_REQUESTING,
        responding = INVOCATION_LIVE_PHASE_RESPONDING,
    )
}

pub(crate) fn invocation_live_phase_sql(invocation_ref: &str) -> String {
    let first_token_measured_sql =
        sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.first_token_ms"));
    invocation_live_phase_sql_with_first_token_predicate(invocation_ref, &first_token_measured_sql)
}

pub(crate) fn invocation_live_phase_sql_with_timing_sql(
    invocation_ref: &str,
    first_token_timing_sql: &str,
) -> String {
    invocation_live_phase_sql_with_timing_sql_at_snapshot(
        invocation_ref,
        first_token_timing_sql,
        None,
    )
}

pub(crate) fn invocation_live_phase_sql_with_timing_sql_at_snapshot(
    invocation_ref: &str,
    first_token_timing_sql: &str,
    attempt_snapshot_id: Option<i64>,
) -> String {
    let first_token_measured_sql =
        sqlite_nonnegative_timing_sql(&format!("({first_token_timing_sql})"));
    invocation_live_phase_sql_with_first_token_predicate_at_snapshot(
        invocation_ref,
        &first_token_measured_sql,
        attempt_snapshot_id,
    )
}
