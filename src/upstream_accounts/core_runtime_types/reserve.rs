use super::*;
use serde::Serialize;

pub(crate) const RESERVE_WIRE_MODEL: &str = "gpt-reserve";
pub(crate) const RESERVE_BINDING_SOURCE_USAGE: &str = "additional_rate_limits";
pub(crate) const RESERVE_AVAILABILITY_AVAILABLE: &str = "available";
pub(crate) const RESERVE_AVAILABILITY_UNAVAILABLE: &str = "unavailable";
pub(crate) const RESERVE_AVAILABILITY_UNKNOWN: &str = "unknown";
pub(crate) const RESERVE_USAGE_SNAPSHOT_MAX_AGE_SECS: i64 = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolQuotaRoute {
    Normal,
    Reserve,
}

impl PoolQuotaRoute {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Reserve => "reserve",
        }
    }

    pub(crate) fn updates_shared_route_health(self) -> bool {
        self == Self::Normal
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedUsageWindow {
    pub(crate) used_percent: f64,
    pub(crate) window_duration_mins: i64,
    pub(crate) resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReserveQuotaSnapshot {
    pub(crate) availability: String,
    pub(crate) allowed: Option<bool>,
    pub(crate) limit_reached: Option<bool>,
    pub(crate) normal_model_slug: Option<String>,
    pub(crate) metered_feature: Option<String>,
    pub(crate) binding_source: Option<String>,
    pub(crate) primary_window: Option<RateWindowSnapshot>,
    pub(crate) secondary_window: Option<RateWindowSnapshot>,
    pub(crate) captured_at: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedReserveQuotaSnapshot {
    pub(crate) availability: String,
    pub(crate) allowed: Option<bool>,
    pub(crate) limit_reached: Option<bool>,
    pub(crate) normal_model_slug: Option<String>,
    pub(crate) metered_feature: Option<String>,
    pub(crate) binding_source: Option<String>,
    pub(crate) primary: Option<NormalizedUsageWindow>,
    pub(crate) secondary: Option<NormalizedUsageWindow>,
    pub(crate) primary_available_percent: Option<f64>,
    pub(crate) secondary_available_percent: Option<f64>,
}
