use super::*;
use serde_json::json;

#[test]
fn reserve_usage_snapshot_accepts_object_additional_rate_limits() {
    let payload = json!({
        "additional_rate_limits": {
            "gpt-reserve": {
                "rate_limit": {
                    "allowed": true,
                    "primary_window": {
                        "used_percent": 12.5,
                        "available_percent": 87.5,
                        "window_duration_mins": 300,
                        "resets_at": "2026-11-01T12:00:00Z"
                    },
                    "secondary_window": {
                        "used_percent": 40,
                        "window_duration_mins": 10080,
                        "resets_at": "2026-11-07T00:00:00Z"
                    }
                }
            }
        }
    });

    let reserve = normalize_usage_snapshot(&payload)
        .expect("normalize object reserve snapshot")
        .reserve
        .expect("reserve snapshot should be present");
    assert_eq!(reserve.availability, RESERVE_AVAILABILITY_AVAILABLE);
    assert_eq!(reserve.primary_available_percent, Some(87.5));
    assert_eq!(
        reserve.primary.as_ref().map(|window| window.used_percent),
        Some(12.5)
    );
    assert_eq!(
        reserve
            .secondary
            .as_ref()
            .and_then(|window| window.resets_at.as_deref()),
        Some("2026-11-07T00:00:00Z")
    );
}

#[test]
fn reserve_usage_snapshot_accepts_array_additional_rate_limits_and_unavailable_state() {
    let payload = json!({
        "additionalRateLimits": [
            {
                "limitName": "gpt-5.4",
                "rateLimit": {"allowed": true}
            },
            {
                "limitName": "gpt-reserve",
                "rateLimit": {
                    "allowed": false,
                    "primaryWindow": {
                        "usedPercent": 100,
                        "windowDurationMins": 300
                    }
                }
            }
        ]
    });

    let reserve = normalize_usage_snapshot(&payload)
        .expect("normalize array reserve snapshot")
        .reserve
        .expect("reserve snapshot should be present");
    assert_eq!(reserve.availability, RESERVE_AVAILABILITY_UNAVAILABLE);
    assert_eq!(
        reserve.primary.as_ref().map(|window| window.used_percent),
        Some(100.0)
    );
}

#[test]
fn reserve_usage_snapshot_keeps_ambiguous_window_data_unknown() {
    let payload = json!({
        "additional_rate_limits": {
            "gpt-reserve": {
                "rate_limit": {
                    "primary_window": {
                        "used_percent": 20,
                        "window_duration_mins": 300
                    }
                }
            }
        }
    });

    let reserve = normalize_usage_snapshot(&payload)
        .expect("normalize ambiguous reserve snapshot")
        .reserve
        .expect("reserve snapshot should be present");
    assert_eq!(reserve.availability, RESERVE_AVAILABILITY_UNKNOWN);
}

#[test]
fn reserve_usage_snapshot_keeps_allowance_without_quota_observation_unknown() {
    let payload = json!({
        "additional_rate_limits": {
            "gpt-reserve": {
                "rate_limit": {
                    "allowed": true
                }
            }
        }
    });

    let reserve = normalize_usage_snapshot(&payload)
        .expect("normalize incomplete reserve snapshot")
        .reserve
        .expect("reserve snapshot should be present");
    assert_eq!(reserve.availability, RESERVE_AVAILABILITY_UNKNOWN);
}

#[test]
fn reserve_usage_snapshot_marks_any_exhausted_window_unavailable() {
    let payload = json!({
        "additional_rate_limits": {
            "gpt-reserve": {
                "rate_limit": {
                    "allowed": true,
                    "primary_window": {
                        "used_percent": 100,
                        "window_duration_mins": 300
                    },
                    "secondary_window": {
                        "used_percent": 20,
                        "window_duration_mins": 10080
                    }
                }
            }
        }
    });

    let reserve = normalize_usage_snapshot(&payload)
        .expect("normalize exhausted reserve snapshot")
        .reserve
        .expect("reserve snapshot should be present");
    assert_eq!(reserve.availability, RESERVE_AVAILABILITY_UNAVAILABLE);
}
