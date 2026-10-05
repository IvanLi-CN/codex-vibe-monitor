use super::*;

pub(crate) fn proxy_stream_usage_observed(response_info: &ResponseCaptureInfo) -> bool {
    has_any_usage_tokens(&response_info.usage)
}

pub(crate) fn proxy_stream_failure_origin_from_usage_reason(
    usage_missing_reason: Option<&str>,
) -> Option<&'static str> {
    let reason = usage_missing_reason?;
    if reason.contains("response_decode_failed:") {
        Some("content_decode")
    } else if reason
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("stream_event_parse_error"))
    {
        Some("stream_parse")
    } else {
        None
    }
}

pub(crate) fn proxy_stream_upstream_read_error_kind(err: &io::Error) -> &'static str {
    if let Some(source) = err.get_ref()
        && let Some(reqwest_err) = source.downcast_ref::<reqwest::Error>()
    {
        if reqwest_err.is_timeout() {
            return "timeout";
        }
        if reqwest_err.is_decode() {
            return "decode";
        }
        if reqwest_err.is_body() {
            return "body";
        }
        if reqwest_err.is_request() {
            return "request";
        }
    }

    match err.kind() {
        io::ErrorKind::TimedOut => "timeout",
        io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData => "decode",
        io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => "connection",
        _ => "other",
    }
}
