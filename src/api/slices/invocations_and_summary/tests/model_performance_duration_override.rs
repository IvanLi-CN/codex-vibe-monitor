#[cfg(test)]
mod model_performance_duration_override_tests {
    use super::*;

    fn group(model: &str, reasoning_effort: Option<&str>) -> UsageBreakdownGroupKey {
        UsageBreakdownGroupKey {
            model: model.to_string(),
            reasoning_effort: reasoning_effort.map(str::to_string),
        }
    }

    #[test]
    fn model_performance_keeps_ttft_only_model_after_post_token_failure() {
        let mut accumulator = ModelPerformanceAccumulator::default();
        accumulator.models.insert(
            group("gpt-5.6-sol", None),
            ModelPerformanceAccumulator {
                first_token_sample_count: 1,
                first_token_sum_ms: 420.0,
                ..Default::default()
            },
        );

        let response = accumulator.into_response(
            ExactUtcRange {
                start: utc_at(2026, 7, 26, 0, 0, 0),
                end: utc_at(2026, 7, 26, 1, 0, 0),
            },
            true,
        );

        assert_eq!(response.models.len(), 1);
        assert_eq!(response.models[0].model, "gpt-5.6-sol");
        assert_eq!(response.models[0].metrics.avg_first_token_ms, Some(420.0));
    }

    fn utc_at(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .expect("valid test time")
    }

    #[test]
    fn wall_clock_duration_dedupes_overlaps_per_account_model_and_total_scopes() {
        let rows = vec![
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 10).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 14).timestamp_millis() as f64,
            },
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 12).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 13).timestamp_millis() as f64,
            },
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(77),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 13).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 17).timestamp_millis() as f64,
            },
        ];

        let overrides = compute_model_performance_duration_overrides(&rows);

        assert_eq!(overrides.total_wall_clock_ms, Some(7_000.0));
        assert_eq!(
            overrides.by_account_wall_clock_ms.get(&Some(42)).copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides.by_account_wall_clock_ms.get(&Some(77)).copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .get(&group("gpt-5.4", Some("high")))
                .copied(),
            Some(7_000.0)
        );
        assert_eq!(
            overrides
                .by_account_group_wall_clock_ms
                .get(&AccountModelGroupKey {
                    upstream_account_id: Some(42),
                    group: group("gpt-5.4", Some("high")),
                })
                .copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides
                .by_account_group_wall_clock_ms
                .get(&AccountModelGroupKey {
                    upstream_account_id: Some(77),
                    group: group("gpt-5.4", Some("high")),
                })
                .copied(),
            Some(4_000.0)
        );
    }

    #[test]
    fn wall_clock_duration_keeps_cross_model_overlap_local_to_each_model_group() {
        let rows = vec![
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: Some("high".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 10).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 14).timestamp_millis() as f64,
            },
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(77),
                model: "gpt-5.6-sol".to_string(),
                reasoning_effort: Some("low".to_string()),
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 13).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 0, 17).timestamp_millis() as f64,
            },
        ];

        let overrides = compute_model_performance_duration_overrides(&rows);

        assert_eq!(overrides.total_wall_clock_ms, Some(7_000.0));
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .get(&group("gpt-5.4", Some("high")))
                .copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .get(&group("gpt-5.6-sol", Some("low")))
                .copied(),
            Some(4_000.0)
        );
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .values()
                .copied()
                .sum::<f64>(),
            8_000.0
        );
    }

    #[test]
    fn wall_clock_duration_clips_tail_to_selected_range() {
        let rows = vec![
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: None,
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 58).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 1, 0).timestamp_millis() as f64,
            },
            SuccessfulBilledUsageDurationIntervalRow {
                upstream_account_id: Some(42),
                model: "gpt-5.4".to_string(),
                reasoning_effort: None,
                start_epoch_ms: utc_at(2026, 7, 15, 12, 0, 59).timestamp_millis() as f64,
                end_epoch_ms: utc_at(2026, 7, 15, 12, 1, 0).timestamp_millis() as f64,
            },
        ];

        let overrides = compute_model_performance_duration_overrides(&rows);

        assert_eq!(overrides.total_wall_clock_ms, Some(2_000.0));
        assert_eq!(
            overrides.by_account_wall_clock_ms.get(&Some(42)).copied(),
            Some(2_000.0)
        );
        assert_eq!(
            overrides
                .by_group_wall_clock_ms
                .get(&group("gpt-5.4", None))
                .copied(),
            Some(2_000.0)
        );
        assert_eq!(
            overrides
                .by_account_group_wall_clock_ms
                .get(&AccountModelGroupKey {
                    upstream_account_id: Some(42),
                    group: group("gpt-5.4", None),
                })
                .copied(),
            Some(2_000.0)
        );
    }
}
