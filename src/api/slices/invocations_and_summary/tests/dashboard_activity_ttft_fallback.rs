#[cfg(test)]
mod dashboard_activity_ttft_fallback_tests {
    use super::*;

    #[test]
    fn residual_archive_fallback_keeps_unassigned_ttft_samples() {
        let account_totals = HashMap::from([(
            Some(7),
            DashboardActivityAccountFallbackTotals {
                first_token_sample_count: 2,
                first_token_sum_ms: 500.0,
                ..DashboardActivityAccountFallbackTotals::default()
            },
        )]);

        assert_eq!(
            dashboard_activity_residual_first_token_totals(3, 900.0, &account_totals),
            (1, 400.0)
        );
    }
}
