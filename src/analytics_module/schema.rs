use serde::{Deserialize, Serialize};

pub(crate) const ANALYTICS_MODULE_VERSION: i64 = 1;
pub(crate) const ANALYTICS_MODULE_READER_VERSION: i64 = 1;
pub(crate) const ANALYTICS_MODULE_DDL_COMPONENT: &str = "analytics-module";
pub(crate) const ANALYTICS_MODULE_DDL_STEP: &str = "module-owner-build-publication-v1";

pub(crate) const ANALYTICS_ROLE_OWNER: &str = "cvm_analytics_owner";
pub(crate) const ANALYTICS_ROLE_READER: &str = "cvm_analytics_reader";
pub(crate) const ANALYTICS_ROLE_APPLICATION: &str = "cvm_analytics_app";
pub(crate) const ANALYTICS_ROLE_MAINTENANCE: &str = "cvm_analytics_maintenance";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnalyticsDdlStep {
    pub(crate) order: i64,
    pub(crate) step_id: &'static str,
    pub(crate) min_reader_version: i64,
    pub(crate) requires_bootstrap: bool,
}

pub(crate) const ANALYTICS_MODULE_DDL_STEPS: &[AnalyticsDdlStep] = &[AnalyticsDdlStep {
    order: 1,
    step_id: ANALYTICS_MODULE_DDL_STEP,
    min_reader_version: ANALYTICS_MODULE_READER_VERSION,
    requires_bootstrap: true,
}];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsModuleId {
    Invocation,
    LongTermUsage,
    ParallelWork,
    UpstreamAccountActivity,
    PromptCacheStatistics,
    DashboardActivity,
    Network,
    ForwardProxyStatistics,
}

impl AnalyticsModuleId {
    pub(crate) const ALL: [Self; 8] = [
        Self::Invocation,
        Self::LongTermUsage,
        Self::ParallelWork,
        Self::UpstreamAccountActivity,
        Self::PromptCacheStatistics,
        Self::DashboardActivity,
        Self::Network,
        Self::ForwardProxyStatistics,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Invocation => "invocation",
            Self::LongTermUsage => "long_term_usage",
            Self::ParallelWork => "parallel_work",
            Self::UpstreamAccountActivity => "upstream_account_activity",
            Self::PromptCacheStatistics => "prompt_cache_statistics",
            Self::DashboardActivity => "dashboard_activity",
            Self::Network => "network",
            Self::ForwardProxyStatistics => "forward_proxy_statistics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsConsumerState {
    Building,
    Active,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsOwnerStatus {
    Fenced,
    Claiming,
    Active,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsBuildState {
    Pending,
    Building,
    Ready,
    Failed,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsPublicationState {
    Staging,
    Published,
    Superseded,
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsRepairState {
    Pending,
    Running,
    Resolved,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsDdlState {
    Applying,
    Applied,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsModuleDeclaration {
    pub(crate) id: AnalyticsModuleId,
    pub(crate) consumer_id: &'static str,
    pub(crate) version: i64,
    pub(crate) reader_version: i64,
    pub(crate) requires_retention_pin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsConsumer {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) consumer_id: String,
    pub(crate) version: i64,
    pub(crate) state: AnalyticsConsumerState,
    pub(crate) source_instance_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) baseline_change_id: i64,
    pub(crate) checkpoint: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) retention_pin_id: Option<String>,
    pub(crate) active_build_id: Option<String>,
    pub(crate) active_publication_id: Option<String>,
    pub(crate) legacy_reader_fenced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsOwner {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) owner_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) status: AnalyticsOwnerStatus,
    pub(crate) build_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsBuild {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) build_id: String,
    pub(crate) consumer_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) source_generation: i64,
    pub(crate) source_high_watermark: i64,
    pub(crate) source_coverage_revision: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) state: AnalyticsBuildState,
    pub(crate) retention_pin_id: Option<String>,
    pub(crate) projection_version: i64,
    pub(crate) error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsPublication {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) publication_id: String,
    pub(crate) consumer_id: String,
    pub(crate) build_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) published_through_change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) projection_version: i64,
    pub(crate) reader_version: i64,
    pub(crate) state: AnalyticsPublicationState,
    pub(crate) legacy_reader_fenced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsRepairObligation {
    pub(crate) repair_id: String,
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) consumer_id: String,
    pub(crate) build_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) from_change_id: i64,
    pub(crate) through_change_id: i64,
    pub(crate) reason_code: String,
    pub(crate) state: AnalyticsRepairState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsDdlLedgerEntry {
    pub(crate) component: String,
    pub(crate) step_order: i64,
    pub(crate) step_id: String,
    pub(crate) checksum: String,
    pub(crate) state: AnalyticsDdlState,
    pub(crate) min_reader_version: i64,
    pub(crate) requires_bootstrap: bool,
    pub(crate) error_code: Option<String>,
}
