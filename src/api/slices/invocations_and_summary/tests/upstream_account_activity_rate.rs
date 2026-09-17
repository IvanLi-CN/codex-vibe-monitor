#[cfg(test)]
mod upstream_account_activity_rate_tests {
    use super::*;

    fn utc_at(epoch: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(epoch, 0)
            .single()
            .expect("valid test epoch")
    }

    #[test]
    fn rates_use_recent_five_minute_active_tail() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 600_000,
                total_tokens: 10_000,
                total_cost: 10.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 720_000,
                total_tokens: 0,
                total_cost: 0.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 780_000,
                total_tokens: 300,
                total_cost: 0.30,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 840_000,
                total_tokens: 100,
                total_cost: 0.10,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (400.0 / (220.0 / 60.0))).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.40 / (220.0 / 60.0))).abs() < 1e-9);
    }

    #[test]
    fn rates_return_zero_when_recent_window_has_no_usage() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![UpstreamAccountRateUsageEvent {
            occurred_at_epoch_ms: 600_000,
            total_tokens: 10_000,
            total_cost: 10.0,
        }];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert_eq!(tokens_per_minute, Some(0.0));
        assert_eq!(spend_rate, Some(0.0));
    }

    #[test]
    fn rates_exclude_events_before_mid_minute_tail_window() {
        let range_start = utc_at(0);
        let range_end = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 5, 30)
            .single()
            .expect("valid range end");
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: Utc
                    .with_ymd_and_hms(2026, 7, 1, 12, 0, 1)
                    .single()
                    .expect("valid excluded event time")
                    .timestamp_millis(),
                total_tokens: 10_000,
                total_cost: 10.0,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: Utc
                    .with_ymd_and_hms(2026, 7, 1, 12, 0, 31)
                    .single()
                    .expect("valid included event time")
                    .timestamp_millis(),
                total_tokens: 100,
                total_cost: 0.10,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (100.0 / 5.0)).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.10 / 5.0)).abs() < 1e-9);
    }

    #[test]
    fn rates_floor_first_active_event_to_minute_boundary() {
        let range_start = utc_at(0);
        let range_end = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 5, 0)
            .single()
            .expect("valid range end");
        let usage = vec![UpstreamAccountRateUsageEvent {
            occurred_at_epoch_ms: Utc
                .with_ymd_and_hms(2026, 7, 1, 12, 4, 59)
                .single()
                .expect("valid late-minute event time")
                .timestamp_millis(),
            total_tokens: 600,
            total_cost: 0.60,
        }];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - 600.0).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - 0.60).abs() < 1e-9);
    }

    #[test]
    fn rates_use_earliest_active_event_when_events_are_newest_first() {
        let range_start = utc_at(600);
        let range_end = utc_at(1_000);
        let usage = vec![
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 900_000,
                total_tokens: 100,
                total_cost: 0.10,
            },
            UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: 780_000,
                total_tokens: 300,
                total_cost: 0.30,
            },
        ];

        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(&usage, range_start, range_end);

        assert!((tokens_per_minute.expect("token rate") - (400.0 / (220.0 / 60.0))).abs() < 1e-9);
        assert!((spend_rate.expect("spend rate") - (0.40 / (220.0 / 60.0))).abs() < 1e-9);
    }
}
