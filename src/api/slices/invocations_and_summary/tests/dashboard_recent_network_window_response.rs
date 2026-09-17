#[cfg(test)]
mod dashboard_recent_network_window_response_tests {
    use super::*;

    #[test]
    fn recent_window_response_preserves_unavailable_gap_points() {
        let response = build_dashboard_recent_network_window_response(
            crate::dashboard_network_speed::DashboardRecentNetworkWindowSnapshot {
                range_start_epoch_second: 1_000,
                range_end_epoch_second: 1_300,
                window_seconds: 300,
                sample_seconds: 1,
                is_warming_up: true,
                points: vec![
                    crate::dashboard_network_speed::DashboardRecentNetworkWindowPointSnapshot {
                        sample_start_epoch_second: 1_000,
                        sample_end_epoch_second: 1_001,
                        totals: DashboardNetworkByteTotals::default(),
                        is_available: false,
                    },
                    crate::dashboard_network_speed::DashboardRecentNetworkWindowPointSnapshot {
                        sample_start_epoch_second: 1_001,
                        sample_end_epoch_second: 1_002,
                        totals: DashboardNetworkByteTotals {
                            upload_bytes: 2_048,
                            download_bytes: 4_096,
                        },
                        is_available: true,
                    },
                ],
            },
        );

        assert!(response.is_warming_up);
        assert_eq!(response.points.len(), 2);
        assert_eq!(response.points[0].upload_bytes_per_second, 0.0);
        assert!(!response.points[0].is_available);
        assert_eq!(response.points[1].upload_bytes_per_second, 2_048.0);
        assert_eq!(response.points[1].download_bytes_per_second, 4_096.0);
        assert!(response.points[1].is_available);
    }
}
