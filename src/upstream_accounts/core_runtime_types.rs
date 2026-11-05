use super::*;

#[path = "core_runtime_types/account_models.rs"]
mod account_models;
#[path = "core_runtime_types/constants.rs"]
mod constants;
#[path = "core_runtime_types/credentials.rs"]
mod credentials;
#[path = "core_runtime_types/oauth_session_jobs.rs"]
mod oauth_session_jobs;
#[path = "core_runtime_types/oauth_token_lifecycle.rs"]
mod oauth_token_lifecycle;
#[path = "core_runtime_types/reserve.rs"]
mod reserve;
#[path = "core_runtime_types/routing_models.rs"]
mod routing_models;
#[path = "core_runtime_types/runtime_coordination.rs"]
mod runtime_coordination;
#[path = "core_runtime_types/runtime_state.rs"]
mod runtime_state;

pub(crate) use account_models::*;
pub(crate) use constants::*;
pub(crate) use credentials::*;
pub(crate) use oauth_session_jobs::*;
pub(crate) use oauth_token_lifecycle::*;
pub(crate) use reserve::*;
pub(crate) use routing_models::*;
pub(crate) use runtime_coordination::*;

#[cfg(test)]
mod status_change_reason_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn default_status_change_reasons_enable_every_reason_code() {
        let settings = default_status_change_reasons();

        assert_eq!(settings.len(), STATUS_CHANGE_REASON_CODES.len());
        for reason_code in STATUS_CHANGE_REASON_CODES {
            assert_eq!(settings.get(reason_code), Some(&true));
        }
    }

    #[test]
    fn legacy_upstream_rejected_alias_maps_to_402_reason() {
        assert_eq!(
            canonical_status_change_reason_code(
                LEGACY_UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_REJECTED,
            ),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
        );
    }

    #[test]
    fn status_change_reason_patch_rejects_legacy_alias_keys() {
        let request = UpdateStatusChangeReasonSettingsRequest {
            values: BTreeMap::from([(
                LEGACY_UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_REJECTED.to_string(),
                Some(false),
            )]),
        };

        assert!(request.validate_keys().is_err());
    }
}
