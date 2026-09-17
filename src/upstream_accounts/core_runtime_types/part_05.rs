#[derive(Debug, Deserialize)]
pub(crate) struct JwtExpiryClaims {
    #[serde(default)]
    pub(crate) exp: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtProfileClaims {
    #[serde(default)]
    pub(crate) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtAuthClaims {
    #[serde(default)]
    pub(crate) chatgpt_plan_type: Option<String>,
    #[serde(default)]
    pub(crate) chatgpt_user_id: Option<String>,
    #[serde(default)]
    pub(crate) user_id: Option<String>,
    #[serde(default)]
    pub(crate) chatgpt_account_id: Option<String>,
}

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
