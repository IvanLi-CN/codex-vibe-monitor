use super::*;

const UPSTREAM_ERROR_CODE_SERVER_ERROR: &str = "server_error";
const RESPONSES_SERVER_ERROR_OVERLOAD_MESSAGE: &str = "our servers are currently overloaded";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolResponsesOverloadRetryPolicy {
    LegacyFailover,
    SameAccountOnly,
}

fn normalize_responses_overload_message(message: Option<&str>) -> String {
    let mut normalized = String::new();
    let mut previous_was_separator = false;
    for character in message.unwrap_or_default().chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator && !normalized.is_empty() {
            normalized.push(' ');
            previous_was_separator = true;
        }
    }
    normalized.trim_end().to_string()
}

pub(crate) fn upstream_error_is_server_error_overload(
    code: Option<&str>,
    message: Option<&str>,
) -> bool {
    code.is_some_and(|value| value.eq_ignore_ascii_case(UPSTREAM_ERROR_CODE_SERVER_ERROR))
        && normalize_responses_overload_message(message)
            .contains(RESPONSES_SERVER_ERROR_OVERLOAD_MESSAGE)
}

pub(crate) fn classify_pool_responses_overload(
    status: StatusCode,
    code: Option<&str>,
    message: Option<&str>,
) -> Option<PoolResponsesOverloadRetryPolicy> {
    if status != StatusCode::OK {
        return None;
    }
    if upstream_error_is_retryable_responses_overload(code, message) {
        return Some(PoolResponsesOverloadRetryPolicy::LegacyFailover);
    }
    upstream_error_is_server_error_overload(code, message)
        .then_some(PoolResponsesOverloadRetryPolicy::SameAccountOnly)
}
