use sha2::{Digest, Sha256};

use super::*;

pub(crate) const LIVE_REQUEST_STREAMING_REVISION: &str = "responses-live-request-body-v2";
pub(crate) const LIVE_REQUEST_STREAMING_MIN_SUCCESS_SAMPLES: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestBodyTransportMode {
    Buffered,
    LiveFirst,
    Unknown,
}

impl RequestBodyTransportMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Buffered => "buffered",
            Self::LiveFirst => "live_first",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveRequestStreamingExperimentVariant {
    Control,
    Treatment,
}

impl LiveRequestStreamingExperimentVariant {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Treatment => "treatment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveRequestStreamingDecision {
    pub(crate) transport_mode: RequestBodyTransportMode,
    pub(crate) revision: Option<&'static str>,
    pub(crate) variant: Option<LiveRequestStreamingExperimentVariant>,
    pub(crate) eligible: bool,
    pub(crate) reason: &'static str,
}

impl LiveRequestStreamingDecision {
    pub(crate) fn buffered(reason: &'static str) -> Self {
        Self {
            transport_mode: RequestBodyTransportMode::Buffered,
            revision: None,
            variant: None,
            eligible: false,
            reason,
        }
    }

    pub(crate) fn collects_metrics(&self) -> bool {
        self.revision == Some(LIVE_REQUEST_STREAMING_REVISION) && self.variant.is_some()
    }
}

/// Deterministically assigns a request cohort before any request body is sent.
/// The assignment deliberately uses the invocation id rather than account id so
/// retries and cross-account failover retain their original cohort.
pub(crate) fn decide_live_request_streaming(
    settings: &LiveRequestStreamingSettings,
    invoke_id: &str,
    target: ProxyCaptureTarget,
    routing_metadata_ready: bool,
    upstream_transform_supported: bool,
) -> LiveRequestStreamingDecision {
    if target != ProxyCaptureTarget::Responses {
        return LiveRequestStreamingDecision::buffered("endpoint_not_supported");
    }
    if !settings.enabled {
        return LiveRequestStreamingDecision::buffered("disabled");
    }

    let variant = if live_request_streaming_bucket(invoke_id) < settings.treatment_percent {
        LiveRequestStreamingExperimentVariant::Treatment
    } else {
        LiveRequestStreamingExperimentVariant::Control
    };

    if !routing_metadata_ready {
        return LiveRequestStreamingDecision {
            transport_mode: RequestBodyTransportMode::Buffered,
            revision: Some(LIVE_REQUEST_STREAMING_REVISION),
            variant: Some(variant),
            eligible: false,
            reason: "routing_metadata_incomplete",
        };
    }
    if !upstream_transform_supported {
        return LiveRequestStreamingDecision {
            transport_mode: RequestBodyTransportMode::Buffered,
            revision: Some(LIVE_REQUEST_STREAMING_REVISION),
            variant: Some(variant),
            eligible: false,
            reason: "upstream_transform_not_supported",
        };
    }

    LiveRequestStreamingDecision {
        transport_mode: match variant {
            LiveRequestStreamingExperimentVariant::Control => RequestBodyTransportMode::Buffered,
            LiveRequestStreamingExperimentVariant::Treatment => RequestBodyTransportMode::LiveFirst,
        },
        revision: Some(LIVE_REQUEST_STREAMING_REVISION),
        variant: Some(variant),
        eligible: true,
        reason: match variant {
            LiveRequestStreamingExperimentVariant::Control => "control",
            LiveRequestStreamingExperimentVariant::Treatment => "treatment",
        },
    }
}

pub(crate) fn live_request_streaming_bucket(invoke_id: &str) -> u8 {
    let mut hasher = Sha256::new();
    hasher.update(invoke_id.as_bytes());
    hasher.update([0]);
    hasher.update(LIVE_REQUEST_STREAMING_REVISION.as_bytes());
    let digest = hasher.finalize();
    u16::from_be_bytes([digest[0], digest[1]]).wrapping_rem(100) as u8
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LiveRequestStreamingMeasurement {
    pub(crate) raw_body_bytes: Option<usize>,
    pub(crate) logical_body_bytes: Option<usize>,
    pub(crate) upstream_request_first_byte_ms: Option<f64>,
    pub(crate) request_body_capture_complete_ms: Option<f64>,
    pub(crate) request_upstream_overlap_ms: Option<f64>,
    pub(crate) first_response_byte_total_ms: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) route_finalization_raw_bytes: Option<usize>,
    pub(crate) route_finalization_logical_bytes: Option<usize>,
    pub(crate) route_finalization_raw_ratio: Option<f64>,
    pub(crate) route_finalization_logical_ratio: Option<f64>,
    pub(crate) route_finalization_ms: Option<f64>,
    pub(crate) route_finalization_outcome: Option<&'static str>,
    pub(crate) route_dependency_factors: Vec<&'static str>,
    pub(crate) routing_hot_cache_hit: Option<bool>,
    pub(crate) routing_hot_cache_cold_load: Option<bool>,
    pub(crate) first_attempt_failed: bool,
    pub(crate) fallback_or_retry: bool,
    pub(crate) capture_failed: bool,
    pub(crate) ambiguous_upstream_delivery: bool,
    pub(crate) upstream_account_group: Option<String>,
    pub(crate) experiment_account_group: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LiveRequestStreamingRiskFlags {
    pub(crate) first_attempt_failed: bool,
    pub(crate) fallback_or_retry: bool,
    pub(crate) ambiguous_upstream_delivery: bool,
}

pub(crate) fn live_request_streaming_risk_flags(
    live_first_attempt_failed: bool,
    upstream_body_started: bool,
    pool_attempt_count: usize,
) -> LiveRequestStreamingRiskFlags {
    let pool_retry = pool_attempt_count > 1;
    LiveRequestStreamingRiskFlags {
        first_attempt_failed: live_first_attempt_failed || pool_retry,
        fallback_or_retry: live_first_attempt_failed || pool_retry,
        ambiguous_upstream_delivery: live_first_attempt_failed && upstream_body_started,
    }
}

pub(crate) fn request_upstream_overlap_ms(
    capture_complete_ms: Option<f64>,
    upstream_first_byte_ms: Option<f64>,
) -> Option<f64> {
    match (capture_complete_ms, upstream_first_byte_ms) {
        (Some(capture_complete_ms), Some(upstream_first_byte_ms)) => {
            Some((capture_complete_ms - upstream_first_byte_ms).max(0.0))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(percent: u8) -> LiveRequestStreamingSettings {
        LiveRequestStreamingSettings {
            enabled: true,
            treatment_percent: percent,
        }
    }

    #[test]
    fn live_first_metrics_assignment_is_stable_when_enabled() {
        let first = decide_live_request_streaming(
            &settings(50),
            "invoke-1",
            ProxyCaptureTarget::Responses,
            true,
            true,
        );
        let second = decide_live_request_streaming(
            &settings(50),
            "invoke-1",
            ProxyCaptureTarget::Responses,
            true,
            true,
        );
        assert_eq!(first, second);
        assert!(first.eligible);
    }

    #[test]
    fn live_first_risk_flags_keep_control_retry_separate_from_ambiguous_delivery() {
        assert_eq!(
            live_request_streaming_risk_flags(false, false, 2),
            LiveRequestStreamingRiskFlags {
                first_attempt_failed: true,
                fallback_or_retry: true,
                ambiguous_upstream_delivery: false,
            }
        );
        assert_eq!(
            live_request_streaming_risk_flags(true, true, 1),
            LiveRequestStreamingRiskFlags {
                first_attempt_failed: true,
                fallback_or_retry: true,
                ambiguous_upstream_delivery: true,
            }
        );
        assert!(!live_request_streaming_risk_flags(true, false, 1).ambiguous_upstream_delivery);
    }

    #[test]
    fn live_first_metrics_respect_control_and_metadata_gate() {
        assert_eq!(
            decide_live_request_streaming(
                &settings(0),
                "invoke-1",
                ProxyCaptureTarget::Responses,
                true,
                true,
            )
            .variant,
            Some(LiveRequestStreamingExperimentVariant::Control)
        );
        let gated = decide_live_request_streaming(
            &settings(100),
            "invoke-1",
            ProxyCaptureTarget::Responses,
            false,
            true,
        );
        assert_eq!(gated.transport_mode, RequestBodyTransportMode::Buffered);
        assert_eq!(gated.reason, "routing_metadata_incomplete");
        assert_eq!(
            request_upstream_overlap_ms(Some(100.0), Some(35.0)),
            Some(65.0)
        );
    }
}
