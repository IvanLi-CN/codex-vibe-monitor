use super::*;

pub(crate) fn upstream_account_name_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("upstreamAccountName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn prompt_cache_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("promptCacheKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Default)]
pub(crate) struct TerminalPayloadMetadata {
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) request_model: Option<String>,
}

pub(crate) fn terminal_payload_metadata(payload: Option<&str>) -> TerminalPayloadMetadata {
    let Some(payload) = payload else {
        return TerminalPayloadMetadata::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return TerminalPayloadMetadata::default();
    };
    let prompt_cache_key = value
        .get("promptCacheKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let upstream_account_id = value.get("upstreamAccountId").and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
    });
    let request_model = value
        .get("requestModel")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string());
    TerminalPayloadMetadata {
        prompt_cache_key,
        upstream_account_id,
        request_model,
    }
}
