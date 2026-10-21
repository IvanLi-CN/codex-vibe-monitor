use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum ForwardProxyValidationErrorCode {
    #[serde(rename = "unsupported_transport")]
    UnsupportedTransport,
    #[serde(rename = "invalid_xhttp_extra")]
    InvalidXhttpExtra,
    #[serde(rename = "unsupported_xhttp_mode")]
    UnsupportedXhttpMode,
    #[serde(rename = "xray_config_error")]
    XrayConfigError,
    #[serde(rename = "xray_startup_error")]
    XrayStartupError,
    #[serde(rename = "proxy_connect_error")]
    ProxyConnectError,
    #[serde(rename = "proxy_timeout")]
    ProxyTimeout,
    #[serde(rename = "upstream_status_rejected")]
    UpstreamStatusRejected,
    #[serde(rename = "validation_failed")]
    ValidationFailed,
}

impl ForwardProxyValidationErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedTransport => "unsupported_transport",
            Self::InvalidXhttpExtra => "invalid_xhttp_extra",
            Self::UnsupportedXhttpMode => "unsupported_xhttp_mode",
            Self::XrayConfigError => "xray_config_error",
            Self::XrayStartupError => "xray_startup_error",
            Self::ProxyConnectError => "proxy_connect_error",
            Self::ProxyTimeout => "proxy_timeout",
            Self::UpstreamStatusRejected => "upstream_status_rejected",
            Self::ValidationFailed => "validation_failed",
        }
    }
}

#[derive(Debug)]
pub(crate) struct ForwardProxyValidationError {
    pub(crate) code: ForwardProxyValidationErrorCode,
    pub(crate) message: String,
}

impl ForwardProxyValidationError {
    pub(crate) fn new(code: ForwardProxyValidationErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ForwardProxyValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ForwardProxyValidationError {}

#[path = "slices/manager_xray_types_and_tests.rs"]
mod manager_xray_types_and_tests;
#[path = "slices/settings_validation_and_runtime_sync.rs"]
mod settings_validation_and_runtime_sync;
#[path = "slices/storage_and_hourly_stats.rs"]
#[expect(
    clippy::too_many_arguments,
    reason = "Forward-proxy statistics adapters preserve established call-site contracts."
)]
mod storage_and_hourly_stats;

pub(crate) use manager_xray_types_and_tests::*;
pub(crate) use settings_validation_and_runtime_sync::*;
pub(crate) use storage_and_hourly_stats::*;
