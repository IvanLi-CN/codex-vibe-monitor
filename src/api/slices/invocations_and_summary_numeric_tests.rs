use super::*;

#[test]
fn summary_live_wait_arithmetic_fails_closed_on_non_finite_values() {
    assert!(matches!(
        summary_live_sum_from_average(Some(f64::MAX), 2),
        Err(ApiError::Unavailable(_))
    ));

    let mut sum = f64::MAX;
    assert!(matches!(
        checked_summary_live_add(&mut sum, f64::MAX, "test wait sum"),
        Err(ApiError::Unavailable(_))
    ));

    let mut rounded = 0.1;
    checked_summary_live_subtract(&mut rounded, 0.1, "test wait subtraction")
        .expect("equal values should subtract cleanly");
    assert_eq!(rounded, 0.0);

    let mut inconsistent = 0.1;
    assert!(matches!(
        checked_summary_live_subtract(&mut inconsistent, 4.0, "test wait subtraction"),
        Err(ApiError::Unavailable(_))
    ));
}

#[test]
fn projection_sql_keeps_success_aliases_and_invocation_identity() {
    assert!(INVOCATION_RESOLVED_FAILURE_CLASS_SQL.contains("'succeeded'"));
    assert!(INVOCATION_RESOLVED_FAILURE_CLASS_SQL.contains("'ok'"));
    let sql = invocation_upstream_account_id_from_attempt_fallback_sql("inv");
    assert!(sql.contains("attempt.occurred_at = inv.occurred_at"));
}
