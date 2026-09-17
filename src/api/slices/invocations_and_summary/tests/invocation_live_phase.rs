#[cfg(test)]
mod invocation_live_phase_tests {
    use super::*;

    #[test]
    fn persisted_live_phase_requires_a_measured_first_token_to_be_responding() {
        let sql = invocation_live_phase_sql("invocation");

        assert!(sql.contains("invocation.first_token_ms IS NOT NULL"));
        assert!(sql.contains("invocation.first_token_ms >= 0"));
        assert!(!sql.contains("t_upstream_ttfb_ms IS NOT NULL"));
        assert!(!sql.contains("t_upstream_stream_ms IS NOT NULL"));
        assert!(!sql.contains("streaming_response"));
    }

    #[test]
    fn zero_millisecond_first_token_is_a_valid_measurement() {
        assert!(has_measured_first_token(Some(0.0)));
        assert!(!has_measured_first_token(Some(-0.1)));
        assert!(!has_measured_first_token(None));
    }

    #[test]
    fn final_attempt_first_token_sql_limits_invocation_timing_to_the_last_attempt() {
        let sql = final_pool_attempt_first_token_ms_sql("attempts", "inv");

        assert!(sql.contains("attempts.id"));
        assert!(sql.contains("final_attempt.invoke_id = attempts.invoke_id"));
        assert!(sql.contains("final_attempt.occurred_at = attempts.occurred_at"));
        assert!(!sql.contains("attempts.status, ''))) IN ('success', 'completed'"));
        assert!(sql.contains("attempts.stream_latency_ms > 0"));
        assert!(sql.contains("typeof(inv.first_token_ms) IN ('integer', 'real')"));
        assert!(sql.contains("inv.first_token_ms = 0"));
        assert!(sql.contains(
            "attempts.status, ''))) NOT IN ('', 'pending', 'running', 'budget_exhausted_final')"
        ));
        assert!(sql.contains("inv.status, ''))) = 'running' AND inv.first_token_ms IS NOT NULL"));
        assert!(sql.contains("attempts.status, ''))) = 'responding'"));
        assert!(sql.contains("attempts.phase, ''))) IN ('responding', 'streaming_response')"));
        assert!(!sql.contains("prior_attempt.attempt_index < attempts.attempt_index"));
        assert!(sql.contains("final_attempt.status, ''))) <> 'budget_exhausted_final'"));
        assert!(sql.contains("ORDER BY final_attempt.attempt_index DESC, final_attempt.id DESC"));
        assert!(sql.contains("THEN CASE WHEN inv.first_token_ms IS NOT NULL"));
        assert!(sql.ends_with("END END AS first_token_ms"));
    }

    #[test]
    fn final_attempt_timing_requires_stream_evidence_for_positive_values() {
        let first_token_sql = final_pool_invocation_timing_sql("inv", "first_token_ms");
        let stream_sql = final_pool_invocation_timing_sql("inv", "t_upstream_stream_ms");

        assert!(first_token_sql.contains("final_attempt.stream_latency_ms > 0"));
        assert!(stream_sql.contains("final_attempt.stream_latency_ms > 0"));
        assert!(!first_token_sql.contains("status, ''))) IN ('success', 'completed'"));
        assert!(!stream_sql.contains("status, ''))) IN ('success', 'completed'"));
    }

    #[test]
    fn invocation_timing_sql_requires_a_real_final_attempt_measurement() {
        let sql = final_pool_invocation_timing_sql("inv", "first_token_ms");

        assert!(sql.contains("final_attempt.first_byte_latency_ms"));
        assert!(sql.contains("final_attempt.stream_latency_ms > 0"));
        assert!(sql.contains("inv.first_token_ms = 0"));
        assert!(sql.contains("inv.first_token_ms >= 0"));
        assert!(sql.contains("ORDER BY final_attempt.attempt_index DESC, final_attempt.id DESC"));
        assert!(sql.contains("THEN inv.first_token_ms END END"));
    }

    #[test]
    fn archive_timing_sql_uses_validated_values_without_pool_attempts() {
        let first_token_sql = invocation_timing_sql_for_source("inv", "first_token_ms", false);
        let stream_sql = invocation_timing_sql_for_source("inv", "t_upstream_stream_ms", false);

        assert!(first_token_sql.contains("inv.first_token_ms >= 0"));
        assert!(stream_sql.contains("inv.t_upstream_stream_ms > 0"));
        assert!(!first_token_sql.contains("pool_upstream_request_attempts"));
        assert!(!stream_sql.contains("pool_upstream_request_attempts"));
    }

    #[test]
    fn persisted_live_phase_sql_uses_final_attempt_ttft() {
        let timing_sql = final_pool_invocation_timing_sql("inv", "first_token_ms");
        let sql = invocation_live_phase_sql_with_timing_sql("inv", &timing_sql);

        assert!(sql.contains("final_attempt.stream_latency_ms > 0"));
        assert!(sql.contains("final_attempt.first_byte_latency_ms"));
        assert!(sql.contains("THEN inv.first_token_ms END END"));
    }
}
